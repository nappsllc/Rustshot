//! Window surface shared by every platform: input events, modifier state,
//! the framebuffer driver trait, and virtual-key constants. Two window
//! kinds share the `Driver` trait: the fullscreen overlay (`run`) and a
//! normal decorated top-level window (`run_window`, for dialogs/Settings).
//! Per-OS implementations live in `wind_win.rs` / `wind_linux.rs` /
//! `wind_macos.rs`.

use crate::pixbuf::PixBuf;

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Mods {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

#[cfg(windows)]
impl Mods {
    pub fn current() -> Self {
        use windows::Win32::UI::Input::KeyboardAndMouse::{
            GetKeyState, VK_CONTROL, VK_MENU, VK_SHIFT,
        };
        unsafe {
            Mods {
                shift: GetKeyState(VK_SHIFT.0 as i32) as u16 & 0x8000 != 0,
                ctrl: GetKeyState(VK_CONTROL.0 as i32) as u16 & 0x8000 != 0,
                alt: GetKeyState(VK_MENU.0 as i32) as u16 & 0x8000 != 0,
            }
        }
    }
}

impl Mods {
    /// The modifiers held now; none in tests (the test machine's keyboard
    /// must not steer them).
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    pub fn current_or_default() -> Self {
        if cfg!(test) { Mods::default() } else { Mods::current() }
    }
}

/// Whether a Win/Super key is held now (`Mods` has no meta state; the
/// Settings window's global-hotkey fields need it). False on macOS.
#[cfg(windows)]
pub fn meta_down() -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_LWIN, VK_RWIN};
    unsafe { [VK_LWIN, VK_RWIN].iter().any(|k| GetKeyState(k.0 as i32) as u16 & 0x8000 != 0) }
}

#[cfg(target_os = "linux")]
pub fn meta_down() -> bool {
    imp::meta_down()
}

#[cfg(target_os = "macos")]
pub fn meta_down() -> bool {
    false
}

#[cfg(target_os = "linux")]
impl Mods {
    pub fn current() -> Self {
        imp::current_mods()
    }
}

#[cfg(target_os = "macos")]
impl Mods {
    pub fn current() -> Self {
        imp::current_mods()
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Ev {
    Move { x: i32, y: i32 },
    Down { x: i32, y: i32 },
    Up { x: i32, y: i32 },
    Wheel { delta: i32, x: i32, y: i32 },
    /// The middle button went down (or up): the Edit window pans with it.
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    Middle { down: bool, x: i32, y: i32 },
    /// The right button went down (or up): the overlay's color ring.
    Right { down: bool, x: i32, y: i32 },
    Key { vk: u32, up: bool, repeat: bool, mods: Mods },
    /// Text input, one UTF-16 code unit per event (WM_CHAR parity on every
    /// backend). Characters outside the BMP arrive as two events, a high
    /// then a low surrogate: text consumers must buffer a high surrogate and
    /// combine it with the next unit (`char::decode_utf16`), dropping an
    /// unpaired one. Control codes (Backspace 0x08, Ctrl+letter, ...) also
    /// arrive here and are for the consumer to filter.
    Char(u16),
    Timer,
    /// `run_window` only: the close button (or Alt+F4 / WM_DELETE_WINDOW)
    /// was pressed. Nothing closes by itself: the driver calls
    /// `wind::close(hwnd)` to end the loop, or ignores it to stay open.
    #[cfg_attr(target_os = "macos", allow(dead_code))] // macOS: no decorated windows yet (main thread only)
    Close,
    /// `run_window` only: new client size in physical pixels. Also sent
    /// once right after `on_create`, so the driver always knows its size.
    #[cfg_attr(target_os = "macos", allow(dead_code))] // macOS: no decorated windows yet (main thread only)
    Resize(u32, u32),
    /// `run_window` only: the window gained (true) or lost keyboard focus.
    #[cfg_attr(target_os = "macos", allow(dead_code))] // macOS: no decorated windows yet (main thread only)
    Focus(bool),
}

/// A normal decorated top-level window for `run_window`.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(target_os = "macos", allow(dead_code))] // macOS: no decorated windows yet (main thread only)
pub struct WindowSpec {
    /// Title bar text (passed through as is).
    pub title: String,
    /// Initial client size in logical pixels (scaled by the monitor's DPI).
    pub w: u32,
    pub h: u32,
    /// Whether the user can resize/maximise the window.
    pub resizable: bool,
    /// Minimum client size in logical pixels (only enforced when resizable).
    pub min: (u32, u32),
    /// Which title-bar buttons the OS frame shows.
    pub controls: Controls,
}

/// The title-bar buttons of a `run_window` window. The title bar itself is
/// the OS's (the design boards: "the OS draws the title bar; everything
/// inside is ours"), so these are the real caption buttons: Snap Layouts,
/// Alt+F4, Win+Up/Down, the system menu and the window manager's own
/// shortcuts all work as in any other app.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(target_os = "macos", allow(dead_code))] // macOS: no decorated windows yet (main thread only)
pub enum Controls {
    /// Minimize, maximize/restore and close: resizable windows (History,
    /// Edit).
    MinMaxClose,
    /// Minimize and close: fixed-size windows (Settings, the launcher, the
    /// update dialog). Windows still shows a disabled maximize button
    /// between them, as for every fixed-size window with a minimize box.
    MinClose,
    /// Close only: modal questions (a dialog to the window manager).
    Close,
}

#[cfg_attr(target_os = "macos", allow(dead_code))]
impl Controls {
    pub fn minimize(self) -> bool {
        self != Controls::Close
    }
    pub fn maximize(self) -> bool {
        self == Controls::MinMaxClose
    }
}

/// `_MOTIF_WM_HINTS` (flags, functions, decorations, input mode, status)
/// for `controls`: the X window manager offers (and draws buttons for)
/// only these actions. Move and close always; minimize unless a dialog;
/// maximize and resize only for a resizable window.
#[cfg(any(target_os = "linux", test))]
pub(crate) fn motif_hints(controls: Controls, resizable: bool) -> [u32; 5] {
    const FUNCTIONS: u32 = 1 << 0;
    const DECORATIONS: u32 = 1 << 1;
    // MWM_FUNC_* / MWM_DECOR_*.
    const F_RESIZE: u32 = 1 << 1;
    const F_MOVE: u32 = 1 << 2;
    const F_MINIMIZE: u32 = 1 << 3;
    const F_MAXIMIZE: u32 = 1 << 4;
    const F_CLOSE: u32 = 1 << 5;
    const D_BORDER: u32 = 1 << 1;
    const D_RESIZEH: u32 = 1 << 2;
    const D_TITLE: u32 = 1 << 3;
    const D_MENU: u32 = 1 << 4;
    const D_MINIMIZE: u32 = 1 << 5;
    const D_MAXIMIZE: u32 = 1 << 6;
    let (min, max) = (controls.minimize(), controls.maximize() && resizable);
    let pick = |on: bool, bit: u32| if on { bit } else { 0 };
    let functions = F_MOVE | F_CLOSE | pick(min, F_MINIMIZE) | pick(max, F_MAXIMIZE) | pick(resizable, F_RESIZE);
    let decorations = D_BORDER | D_TITLE | D_MENU | pick(min, D_MINIMIZE) | pick(max, D_MAXIMIZE) | pick(resizable, D_RESIZEH);
    [FUNCTIONS | DECORATIONS, functions, decorations, 0, 0]
}

/// Colours for the OS-drawn title bar and window border, so the caption
/// reads as part of the design surface under it (`Driver::frame_colors`).
/// Windows 11: DWM immersive dark mode plus caption and border colours
/// (Windows 10: dark mode only); Linux: a dark/light hint to the window
/// manager (`_GTK_THEME_VARIANT`); macOS: a transparent title bar over the
/// window background in the matching appearance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub struct FrameColors {
    pub dark: bool,
    /// Title bar background (`[r, g, b]`): the window's top surface.
    pub caption: [u8; 3],
    /// The 1 px window border.
    pub border: [u8; 3],
}

/// Logical size → physical pixels at `scale` (never below 1 px).
#[cfg_attr(target_os = "macos", allow(dead_code))] // macOS: no decorated windows yet (main thread only)
pub(crate) fn to_phys(v: u32, scale: f32) -> u32 {
    ((v as f32 * scale).round() as u32).max(1)
}

/// Top-left that centres a `w`×`h` outer rect inside `area` (x, y, w, h),
/// clamped so the title bar never starts above/left of the area.
#[cfg_attr(target_os = "macos", allow(dead_code))] // macOS: no decorated windows yet (main thread only)
pub(crate) fn centre_in(area: (i32, i32, i32, i32), w: i32, h: i32) -> (i32, i32) {
    let (ax, ay, aw, ah) = area;
    (ax + ((aw - w) / 2).max(0), ay + ((ah - h) / 2).max(0))
}

/// Top-left that keeps a `w`×`h` outer rect at `(x, y)` inside `area`
/// (x, y, w, h): moved only as far as needed; the top-left wins when it
/// does not fit.
#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
pub(crate) fn keep_inside(area: (i32, i32, i32, i32), x: i32, y: i32, w: i32, h: i32) -> (i32, i32) {
    let (ax, ay, aw, ah) = area;
    (x.min(ax + aw - w).max(ax), y.min(ay + ah - h).max(ay))
}

/// For a `run_window` driver: the scale to draw at, and a resize to the
/// logical `w`×`h` when it differs from the one the window was sized for
/// (`sized_for`, 0 = not yet known): the UI size changed elsewhere (the
/// overlay's Ctrl+=) or the monitor's DPI did.
pub fn follow_scale(hwnd: Hwnd, sized_for: &mut f32, w: u32, h: u32) -> f32 {
    let k = scale(hwnd);
    if *sized_for == 0.0 {
        *sized_for = k;
    } else if (k - *sized_for).abs() > 1e-3 {
        *sized_for = k;
        resize(hwnd, w, h);
    }
    k
}

/// Window icon as `_NET_WM_ICON` data: `[w, h, ARGB pixels...]` decoded from
/// an embedded PNG (any 8-bit color type).
#[cfg(any(target_os = "linux", test))]
pub(crate) fn icon_argb(png_bytes: &[u8]) -> Option<Vec<u32>> {
    let im = crate::pngfile::decode(png_bytes).ok()?;
    let mut out = Vec::with_capacity(2 + (im.w * im.h) as usize);
    out.extend([im.w, im.h]);
    out.extend(im.rgba.as_chunks::<4>().0.iter().map(|p| (p[3] as u32) << 24 | (p[0] as u32) << 16 | (p[1] as u32) << 8 | p[2] as u32));
    Some(out)
}

/// `Xft.dpi` from an X resource-manager string (`XResourceManagerString`)
/// as a scale factor (96 dpi = 1.0); `None` when absent or nonsensical.
#[cfg(any(target_os = "linux", test))]
pub(crate) fn xft_scale(resources: &str) -> Option<f32> {
    resources.lines().find_map(|l| {
        let v = l.strip_prefix("Xft.dpi:")?.trim().parse::<f32>().ok()?;
        (48.0..=960.0).contains(&v).then_some(v / 96.0)
    })
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cursor {
    Arrow,
    Cross,
    IBeam,
    SizeNS,
    SizeWE,
    SizeNWSE,
    SizeNESW,
    Move,
}

#[cfg(windows)]
pub type Hwnd = windows::Win32::Foundation::HWND;

#[cfg(target_os = "linux")]
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Hwnd(pub u64);

#[cfg(target_os = "macos")]
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Hwnd(pub usize);

pub trait Driver {
    /// Window created; stash the handle.
    fn on_create(&mut self, hwnd: Hwnd);
    /// Input/timer event (client coordinates). Returns whether the event
    /// needs a repaint. Backends invalidate after every input event (as a
    /// wndproc would) but after `Ev::Timer` only when this is true, so idle
    /// ticks cost no frame; mouse moves invalidate via `invalidate` instead.
    fn on_event(&mut self, ev: Ev) -> bool;
    /// Compose the current frame as unpremultiplied RGBA, top-down, into a
    /// buffer the driver keeps across frames (no per-frame allocation).
    ///
    /// The buffer is handed out mutably because the presenter converts it
    /// in place (R/B swap) instead of keeping a frame-sized staging copy.
    /// Contract: the driver fully rewrites it on every call, and backends
    /// present each returned frame at most once (every paint path calls
    /// `frame()` first), so its contents after presenting are dead.
    fn frame(&mut self) -> Option<&mut PixBuf>;
    /// What to repaint for the current state, asked wherever a repaint is
    /// requested: rects `[x0, y0, x1, y1]` (client coordinates, end
    /// exclusive; empty = nothing changed), or `None` for the whole window
    /// (the software renderer).
    fn damage(&mut self) -> Option<Vec<[i32; 4]>> {
        None
    }
    /// Windows GDI renderer: paint `rects` of the window into `hdc`
    /// directly. `false` = not handled (present `frame()` instead).
    #[cfg(windows)]
    fn paint(&mut self, _hdc: windows::Win32::Graphics::Gdi::HDC, _rects: &[[i32; 4]]) -> bool {
        false
    }
    /// Cursor for WM_SETCURSOR.
    fn cursor(&self) -> Cursor;
    /// `run_window` only: the title bar and border colours for the current
    /// theme (asked before every paint, applied when they change), or
    /// `None` for the OS defaults.
    fn frame_colors(&self) -> Option<FrameColors> {
        None
    }
    /// Window is being destroyed (loop ends after this).
    fn on_quit(&mut self) {}
}

/// Virtual-key codes (Win32 numbering, shared by every platform backend).
pub mod key {
    pub const BACK: u32 = 0x08;
    pub const TAB: u32 = 0x09;
    pub const RETURN: u32 = 0x0D;
    pub const ESCAPE: u32 = 0x1B;
    pub const SPACE: u32 = 0x20;
    pub const PAGEUP: u32 = 0x21;
    pub const PAGEDOWN: u32 = 0x22;
    pub const END: u32 = 0x23;
    pub const HOME: u32 = 0x24;
    pub const LEFT: u32 = 0x25;
    pub const UP: u32 = 0x26;
    pub const RIGHT: u32 = 0x27;
    pub const DOWN: u32 = 0x28;
    pub const DELETE: u32 = 0x2E;
    pub const INSERT: u32 = 0x2D;
    pub const PRINTSCREEN: u32 = 0x2C;
    /// VK_NUMPAD0 (the keypad digits follow: 0x60..=0x69).
    pub const NUMPAD0: u32 = 0x60;
    /// VK_ADD / VK_SUBTRACT: the keypad's + and -.
    pub const ADD: u32 = 0x6B;
    pub const SUBTRACT: u32 = 0x6D;
    /// VK_OEM_PLUS: the =/+ key (US layout; the + key elsewhere).
    pub const EQUALS: u32 = 0xBB;
    /// VK_OEM_MINUS: the -/_ key.
    pub const MINUS: u32 = 0xBD;
    /// VK_OEM_4 / VK_OEM_6: the [ and ] keys (US layout).
    pub const LBRACKET: u32 = 0xDB;
    pub const RBRACKET: u32 = 0xDD;
}

#[cfg(windows)]
#[path = "wind_win.rs"]
mod imp;
#[cfg(target_os = "linux")]
#[path = "wind_linux.rs"]
mod imp;
#[cfg(target_os = "macos")]
#[path = "wind_macos.rs"]
mod imp;

pub use imp::*;

/// Request a repaint of whatever `drv` reports as damaged.
pub fn request(hwnd: Hwnd, drv: &mut dyn Driver) {
    match drv.damage() {
        None => invalidate(hwnd),
        #[cfg(windows)]
        Some(rects) => imp::invalidate_rects(hwnd, &rects),
        #[cfg(not(windows))]
        Some(rects) => {
            if !rects.is_empty() {
                invalidate(hwnd)
            }
        }
    }
}

/// Idle cadence (hotkey/upload polling, caret blink).
pub const SLOW_TICK_MS: u64 = 150;
/// Cadence while an animation runs.
pub const FAST_TICK_MS: u64 = 16;

/// Per-window timer cadence: the windows currently on the fast tick (keyed
/// by raw handle). Every window starts slow, so the overlay and a dialog on
/// another thread switch independently.
#[derive(Default, Debug)]
pub(crate) struct Cadence {
    fast: Vec<u64>,
}

impl Cadence {
    pub(crate) const fn new() -> Self {
        Cadence { fast: Vec::new() }
    }

    /// Record `on` for `win`; true when its cadence changed.
    pub(crate) fn set(&mut self, win: u64, on: bool) -> bool {
        let pos = self.fast.iter().position(|&w| w == win);
        match (pos, on) {
            (None, true) => self.fast.push(win),
            (Some(i), false) => {
                self.fast.swap_remove(i);
            }
            _ => return false,
        }
        true
    }

    pub(crate) fn tick_ms(&self, win: u64) -> u64 {
        if self.fast.contains(&win) { FAST_TICK_MS } else { SLOW_TICK_MS }
    }

    /// The window is gone (its handle may be reused): drop its entry.
    pub(crate) fn forget(&mut self, win: u64) {
        self.set(win, false);
    }
}

static CADENCE: std::sync::Mutex<Cadence> = std::sync::Mutex::new(Cadence::new());

fn cadence() -> std::sync::MutexGuard<'static, Cadence> {
    CADENCE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Raw handle as the cadence key.
fn win_key(hwnd: Hwnd) -> u64 {
    #[cfg(windows)]
    {
        hwnd.0 as usize as u64
    }
    #[cfg(target_os = "linux")]
    {
        hwnd.0
    }
    #[cfg(target_os = "macos")]
    {
        hwnd.0 as u64
    }
}

/// Current timer cadence of `hwnd` (each window has its own).
pub fn tick_ms(hwnd: Hwnd) -> u64 {
    cadence().tick_ms(win_key(hwnd))
}

/// Switch `hwnd`'s timer between 16 ms (animating) and 150 ms (idle);
/// other windows keep their cadence.
pub fn set_fast_timer(hwnd: Hwnd, on: bool) {
    let changed = cadence().set(win_key(hwnd), on);
    if changed {
        imp::retime(hwnd, tick_ms(hwnd));
    }
}

/// Backends call this when a window is destroyed.
pub(crate) fn forget_tick(hwnd: Hwnd) {
    cadence().forget(win_key(hwnd));
}

/// Test helper: drives `inner` and closes its window once it has drawn a
/// frame and `ms` have passed since it was created (smoke tests of the
/// real Settings window and update dialog).
#[cfg(all(test, not(target_os = "macos")))]
pub(crate) struct AutoClose<'a> {
    inner: &'a mut dyn Driver,
    hwnd: Hwnd,
    ms: u64,
    at: Option<std::time::Instant>,
    pub frames: u32,
}

#[cfg(all(test, not(target_os = "macos")))]
impl<'a> AutoClose<'a> {
    pub fn new(inner: &'a mut dyn Driver, ms: u64) -> Self {
        AutoClose { inner, hwnd: Hwnd::default(), ms, at: None, frames: 0 }
    }
}

#[cfg(all(test, not(target_os = "macos")))]
impl Driver for AutoClose<'_> {
    fn on_create(&mut self, hwnd: Hwnd) {
        self.hwnd = hwnd;
        self.at = Some(std::time::Instant::now() + std::time::Duration::from_millis(self.ms));
        self.inner.on_create(hwnd);
    }
    fn on_event(&mut self, ev: Ev) -> bool {
        let timer = matches!(ev, Ev::Timer);
        let r = self.inner.on_event(ev);
        if timer && self.frames > 0 && self.at.is_some_and(|t| std::time::Instant::now() >= t) {
            close(self.hwnd);
        }
        r
    }
    fn frame(&mut self) -> Option<&mut PixBuf> {
        let f = self.inner.frame();
        if f.is_some() {
            self.frames += 1;
        }
        f
    }
    fn damage(&mut self) -> Option<Vec<[i32; 4]>> {
        self.inner.damage()
    }
    #[cfg(windows)]
    fn paint(&mut self, hdc: windows::Win32::Graphics::Gdi::HDC, rects: &[[i32; 4]]) -> bool {
        let done = self.inner.paint(hdc, rects);
        if done {
            self.frames += 1;
        }
        done
    }
    fn cursor(&self) -> Cursor {
        self.inner.cursor()
    }
    fn frame_colors(&self) -> Option<FrameColors> {
        self.inner.frame_colors()
    }
    fn on_quit(&mut self) {
        self.inner.on_quit();
    }
}

/// Serialises the tests that open windows: they share the process-wide
/// timer cadence and (Windows) GDI/USER object counts.
#[cfg(test)]
pub(crate) fn test_window_lock() -> std::sync::MutexGuard<'static, ()> {
    static WINDOWS: std::sync::Mutex<()> = std::sync::Mutex::new(());
    WINDOWS.lock().unwrap_or_else(|e| e.into_inner())
}

/// Whether a window can be opened here (Linux: `$DISPLAY` is set); the
/// window smoke tests return early without one.
#[cfg(all(test, not(target_os = "macos")))]
pub(crate) fn has_display() -> bool {
    !cfg!(target_os = "linux") || std::env::var_os("DISPLAY").is_some_and(|d| !d.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A window that grows stays inside its monitor, moving only as far as
    /// it takes.
    #[test]
    fn keep_inside_moves_only_as_needed() {
        let mon = (1920, 0, 1920, 1080);
        assert_eq!(keep_inside(mon, 2000, 100, 400, 300), (2000, 100), "fits: kept");
        assert_eq!(keep_inside(mon, 3600, 100, 400, 300), (3440, 100), "right edge");
        assert_eq!(keep_inside(mon, 2000, 900, 400, 300), (2000, 780), "bottom edge");
        assert_eq!(keep_inside(mon, 1900, -20, 400, 300), (1920, 0), "left and top");
        assert_eq!(keep_inside(mon, 2000, 100, 3000, 2000), (1920, 0), "too big: top-left wins");
    }

    struct QuitSoon {
        created: bool,
        fb: PixBuf,
    }

    impl Driver for QuitSoon {
        fn on_create(&mut self, hwnd: Hwnd) {
            self.created = true;
            show_at(hwnd, 0, 0, 320, 240);
            close(hwnd);
        }
        fn on_event(&mut self, _ev: Ev) -> bool {
            false
        }
        fn frame(&mut self) -> Option<&mut PixBuf> {
            Some(&mut self.fb)
        }
        fn cursor(&self) -> Cursor {
            Cursor::Arrow
        }
    }

    #[test]
    #[ignore = "live display access"]
    fn live_window_lifecycle() {
        let _guard = test_window_lock();
        let mut d = QuitSoon { created: false, fb: PixBuf::new(320, 240) };
        let code = run(&mut d);
        assert!(d.created);
        assert_eq!(code, 0);
    }

    /// The buttons each window kind asks the X window manager for.
    #[test]
    fn motif_hints_per_controls() {
        let [flags, f, d, ..] = motif_hints(Controls::MinMaxClose, true);
        assert_eq!(flags, 0b11, "functions and decorations given");
        assert_eq!(f, 0b11_1110, "resize, move, minimize, maximize, close");
        assert_eq!(d, 0b111_1110, "border, resize handles, title, menu, minimize, maximize");
        let [_, f, d, ..] = motif_hints(Controls::MinClose, false);
        assert_eq!(f, 0b10_1100, "move, minimize, close: no resize, no maximize");
        assert_eq!(d, 0b011_1010, "border, title, menu, minimize");
        let [_, f, d, ..] = motif_hints(Controls::Close, false);
        assert_eq!(f, 0b10_0100, "move and close only");
        assert_eq!(d, 0b001_1010);
        // Maximize needs a resizable window.
        assert_eq!(motif_hints(Controls::MinMaxClose, false)[1], 0b10_1100);
        assert!(Controls::MinMaxClose.minimize() && Controls::MinMaxClose.maximize());
        assert!(Controls::MinClose.minimize() && !Controls::MinClose.maximize());
        assert!(!Controls::Close.minimize() && !Controls::Close.maximize());
    }

    #[test]
    fn logical_to_physical_and_centring() {
        assert_eq!(to_phys(400, 1.0), 400);
        assert_eq!(to_phys(400, 1.5), 600);
        assert_eq!(to_phys(301, 1.25), 376);
        assert_eq!(to_phys(0, 2.0), 1);
        assert_eq!(centre_in((0, 0, 1920, 1040), 400, 300), (760, 370));
        assert_eq!(centre_in((-1920, 0, 1920, 1080), 400, 300), (-1160, 390));
        // Larger than the work area: pinned to its top-left.
        assert_eq!(centre_in((100, 50, 800, 600), 1000, 700), (100, 50));
    }

    #[test]
    fn window_icon_png_decodes_to_argb() {
        let d = icon_argb(include_bytes!("../packaging/icons/rustshot-48.png")).expect("decodes");
        assert_eq!(&d[..2], &[48, 48]);
        assert_eq!(d.len(), 2 + 48 * 48);
        assert!(d[2..].iter().any(|p| p >> 24 == 0xff), "has opaque pixels");
        assert!(icon_argb(b"not a png").is_none());
    }

    #[test]
    fn xft_dpi_parses() {
        assert_eq!(xft_scale("Xft.antialias:	1
Xft.dpi:	144
"), Some(1.5));
        assert_eq!(xft_scale("Xft.dpi: 96"), Some(1.0));
        assert_eq!(xft_scale("Xcursor.size: 24
"), None);
        assert_eq!(xft_scale("Xft.dpi:	abc
"), None);
        assert_eq!(xft_scale("Xft.dpi:	0
"), None);
    }

    /// Interactive (X11 / macOS main thread): a 400x300 window rendering a
    /// filled rect; Esc or the close button closes it, as does
    /// `RUSTSHOT_TEST_AUTOCLOSE_MS`. Windows has its own variant in
    /// wind_win.rs (posted WM_CLOSE + pixel readback).
    /// `cargo test interactive_window -- --ignored --nocapture --test-threads=1`
    #[cfg(not(windows))]
    #[test]
    #[ignore = "interactive: opens a window"]
    fn interactive_window_renders_rect() {
        struct Rect {
            hwnd: Hwnd,
            fb: PixBuf,
            deadline: Option<std::time::Instant>,
            painted: bool,
        }
        impl Driver for Rect {
            fn on_create(&mut self, hwnd: Hwnd) {
                self.hwnd = hwnd;
                println!("created, scale {}", scale(hwnd));
            }
            fn on_event(&mut self, ev: Ev) -> bool {
                match ev {
                    Ev::Resize(w, h) => {
                        println!("resize {w}x{h}");
                        self.fb = PixBuf::new(w, h);
                    }
                    Ev::Focus(f) => println!("focus {f}"),
                    Ev::Key { vk: key::ESCAPE, up: false, .. } | Ev::Close => close(self.hwnd),
                    Ev::Timer if self.painted && self.deadline.is_some_and(|d| std::time::Instant::now() >= d) => {
                        close(self.hwnd)
                    }
                    _ => {}
                }
                false
            }
            fn frame(&mut self) -> Option<&mut PixBuf> {
                let (w, h) = self.fb.dimensions();
                for (i, p) in self.fb.as_raw_mut().as_chunks_mut::<4>().0.iter_mut().enumerate() {
                    let (x, y) = (i as u32 % w, i as u32 / w);
                    let inside = x >= w / 4 && x < w * 3 / 4 && y >= h / 4 && y < h * 3 / 4;
                    *p = if inside { [64, 128, 255, 255] } else { [24, 24, 28, 255] };
                }
                self.painted = true;
                Some(&mut self.fb)
            }
            fn cursor(&self) -> Cursor {
                Cursor::Arrow
            }
        }
        let deadline = std::env::var("RUSTSHOT_TEST_AUTOCLOSE_MS")
            .ok()
            .and_then(|v| v.parse().ok())
            .map(|ms| std::time::Instant::now() + std::time::Duration::from_millis(ms));
        let mut d = Rect { hwnd: Hwnd::default(), fb: PixBuf::new(1, 1), deadline, painted: false };
        let spec = WindowSpec {
            title: "rustshot window test".into(),
            w: 400,
            h: 300,
            resizable: true,
            min: (200, 150),
            controls: Controls::MinMaxClose,
        };
        let _guard = test_window_lock();
        run_window(spec, &mut d).expect("window loop");
        assert!(d.painted);
    }

    #[test]
    fn tick_switches_between_fast_and_slow() {
        set_fast_timer(Hwnd::default(), false);
        assert_eq!(tick_ms(Hwnd::default()), SLOW_TICK_MS);
        set_fast_timer(Hwnd::default(), true);
        assert_eq!(tick_ms(Hwnd::default()), FAST_TICK_MS);
        set_fast_timer(Hwnd::default(), false);
        assert_eq!(tick_ms(Hwnd::default()), SLOW_TICK_MS);
    }

    /// The overlay and a dialog toggle their cadence independently.
    #[test]
    fn cadence_is_per_window() {
        let (overlay, dialog) = (0x1001, 0x2002);
        let mut c = Cadence::new();
        assert_eq!((c.tick_ms(overlay), c.tick_ms(dialog)), (SLOW_TICK_MS, SLOW_TICK_MS));
        assert!(c.set(overlay, true), "slow -> fast is a change");
        assert!(!c.set(overlay, true), "fast -> fast is not");
        assert_eq!((c.tick_ms(overlay), c.tick_ms(dialog)), (FAST_TICK_MS, SLOW_TICK_MS));
        assert!(!c.set(dialog, false), "the dialog was already slow");
        assert_eq!(c.tick_ms(overlay), FAST_TICK_MS, "a dialog going idle leaves the overlay fast");
        assert!(c.set(dialog, true));
        assert!(c.set(overlay, false));
        assert_eq!((c.tick_ms(overlay), c.tick_ms(dialog)), (SLOW_TICK_MS, FAST_TICK_MS));
        c.forget(dialog);
        assert_eq!(c.tick_ms(dialog), SLOW_TICK_MS, "a reused handle starts slow");
        c.forget(dialog);
        assert_eq!(c.fast.len(), 0);
    }
}
