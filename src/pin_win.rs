//! Windows: each pin a layered, top-most tool window (no taskbar button)
//! on the UI thread, drawn with `UpdateLayeredWindow` (the opacity is the
//! layer's constant alpha, so Ctrl+wheel redraws nothing). It takes the
//! keyboard when shown or clicked (Esc closes it), a drag moves it, the
//! wheel zooms it about the pointer, a double-click closes it and the
//! right-click menu is a native popup menu. Pins are not kept out of
//! screen capture: they are the user's content, a later capture may want
//! them.

use super::{apply, dragged, menu, render, resample, scaled_size, zoom_about, zoom_step, Cmd, Entry, Frame, View};
use crate::pixbuf::PixBuf;
use crate::rec_ui::Dib;
use crate::theme::Theme;
use std::cell::RefCell;
use std::sync::Arc;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, ReleaseCapture, SetCapture, VK_CONTROL, VK_ESCAPE, VK_SHIFT};
use windows::Win32::UI::WindowsAndMessaging::*;

const CLASS: PCWSTR = w!("rustshot_pin");
const MK_CONTROL: usize = 0x0008;

struct Pin {
    hwnd: HWND,
    img: Arc<PixBuf>,
    th: Theme,
    s: f32,
    view: View,
    /// The image's top-left on screen (physical px).
    origin: (i32, i32),
    dib: Dib,
    /// The window's pixels at the current zoom, and their size.
    px: Vec<u8>,
    size: (u32, u32),
    /// A drag: the cursor at the press and the origin then.
    drag: Option<((i32, i32), (i32, i32))>,
    /// Wheel delta short of a notch.
    wheel: i32,
}

impl Pin {
    fn zoomed(&self) -> (u32, u32) {
        scaled_size(self.img.dimensions(), self.view.zoom)
    }

    fn centre(&self) -> (i32, i32) {
        let (w, h) = self.zoomed();
        (self.origin.0 + w as i32 / 2, self.origin.1 + h as i32 / 2)
    }

    /// Render at the current zoom and show.
    fn redraw(&mut self) {
        let (w, h) = self.zoomed();
        let (ww, wh, px) = if (w, h) == self.img.dimensions() {
            render(&self.img, &self.th, self.s)
        } else {
            render(&resample(&self.img, w, h), &self.th, self.s)
        };
        self.px = px;
        self.size = (ww, wh);
        self.show();
    }

    /// Put the rendered pixels up at the origin and opacity.
    fn show(&mut self) {
        let (x, y) = Frame::at(self.s).window_pos(self.origin);
        self.dib.put_alpha(self.hwnd, x, y, self.size.0, self.size.1, &self.px, self.view.alpha());
    }

    /// Move the window to the origin (a drag: nothing redrawn).
    fn place(&self) {
        let (x, y) = Frame::at(self.s).window_pos(self.origin);
        unsafe {
            let _ = SetWindowPos(self.hwnd, None, x, y, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
        }
    }

    fn set_view(&mut self, v: View, about: (i32, i32)) {
        let z0 = self.view.zoom;
        self.view = v;
        if v.zoom != z0 {
            self.origin = zoom_about(self.origin, about, z0, v.zoom);
            self.redraw();
        } else {
            self.show();
        }
    }
}

thread_local! {
    static PINS: RefCell<Vec<Pin>> = const { RefCell::new(Vec::new()) };
}

fn with_pin<R>(hwnd: HWND, f: impl FnOnce(&mut Pin) -> R) -> Option<R> {
    PINS.with(|p| {
        let mut p = p.try_borrow_mut().ok()?;
        p.iter_mut().find(|p| p.hwnd == hwnd).map(f)
    })
}

fn report() {
    super::set_count(PINS.with(|p| p.borrow().len()));
}

fn cursor() -> (i32, i32) {
    let mut p = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut p);
    }
    (p.x, p.y)
}

fn lp_xy(lp: LPARAM) -> (i32, i32) {
    ((lp.0 & 0xFFFF) as u16 as i16 as i32, ((lp.0 >> 16) & 0xFFFF) as u16 as i16 as i32)
}

fn key_down(vk: u16) -> bool {
    unsafe { GetKeyState(vk as i32) < 0 }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_LBUTTONDOWN => {
                SetCapture(hwnd);
                let c = cursor();
                with_pin(hwnd, |p| p.drag = Some((c, p.origin)));
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                let c = cursor();
                with_pin(hwnd, |p| {
                    if let Some((press, o)) = p.drag {
                        let next = dragged(o, press, c);
                        if next != p.origin {
                            p.origin = next;
                            p.place();
                        }
                    }
                });
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                with_pin(hwnd, |p| p.drag = None);
                let _ = ReleaseCapture();
                LRESULT(0)
            }
            WM_CAPTURECHANGED => {
                with_pin(hwnd, |p| p.drag = None);
                LRESULT(0)
            }
            WM_LBUTTONDBLCLK => {
                close(hwnd);
                LRESULT(0)
            }
            WM_MOUSEWHEEL => {
                let delta = ((wp.0 >> 16) & 0xFFFF) as u16 as i16 as i32;
                wheel(hwnd, delta, wp.0 & MK_CONTROL != 0, lp_xy(lp));
                LRESULT(0)
            }
            WM_KEYDOWN => {
                let vk = wp.0 as u16;
                let ctrl = key_down(VK_CONTROL.0);
                if vk == VK_ESCAPE.0 {
                    close(hwnd);
                } else if ctrl && vk == b'C' as u16 {
                    run(hwnd, Cmd::Copy);
                } else if ctrl && vk == b'S' as u16 {
                    run(hwnd, if key_down(VK_SHIFT.0) { Cmd::SaveAs } else { Cmd::Save });
                } else if ctrl && vk == b'0' as u16 {
                    run(hwnd, Cmd::Zoom100);
                }
                LRESULT(0)
            }
            WM_CONTEXTMENU => {
                // From the keyboard (Shift+F10, the menu key): at the cursor.
                let at = if lp.0 == -1 { cursor() } else { lp_xy(lp) };
                context_menu(hwnd, at);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

/// `delta` of wheel (120 a notch) with the pointer at `at`: zoom about it,
/// or with Ctrl the opacity.
fn wheel(hwnd: HWND, delta: i32, ctrl: bool, at: (i32, i32)) {
    with_pin(hwnd, |p| {
        p.wheel += delta;
        let notches = p.wheel / 120;
        p.wheel %= 120;
        if notches == 0 {
            return;
        }
        let v = if ctrl {
            View { opacity: super::opacity_step(p.view.opacity, notches), ..p.view }
        } else {
            View { zoom: zoom_step(p.view.zoom, notches), ..p.view }
        };
        if v != p.view {
            p.set_view(v, at);
        }
    });
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

/// Append `entries` to popup menu `h` (the Opacity submenu as a popup).
fn build(h: HMENU, entries: &[Entry]) {
    unsafe {
        for e in entries {
            match e {
                Entry::Item { cmd, label, checked, enabled } => {
                    let mut f = MF_STRING;
                    if *checked {
                        f |= MF_CHECKED;
                    }
                    if !*enabled {
                        f |= MF_GRAYED;
                    }
                    let t = wide(label);
                    let _ = AppendMenuW(h, f, cmd.id() as usize, PCWSTR(t.as_ptr()));
                }
                Entry::Sub { label, items } => {
                    let Ok(sub) = CreatePopupMenu() else { continue };
                    build(sub, items);
                    let t = wide(label);
                    let _ = AppendMenuW(h, MF_POPUP, sub.0 as usize, PCWSTR(t.as_ptr()));
                }
                Entry::Sep => {
                    let _ = AppendMenuW(h, MF_SEPARATOR, 0, PCWSTR::null());
                }
            }
        }
    }
}

/// The right-click menu at `at` (screen px); carries out the choice.
fn context_menu(hwnd: HWND, at: (i32, i32)) {
    let Some(view) = with_pin(hwnd, |p| p.view) else { return };
    let n = PINS.with(|p| p.borrow().len());
    let cmd = unsafe {
        let Ok(h) = CreatePopupMenu() else { return };
        build(h, &menu(view, n));
        // The menu closes on a click elsewhere only when its owner is
        // the foreground window.
        let _ = SetForegroundWindow(hwnd);
        let id = TrackPopupMenu(h, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY, at.0, at.1, None, hwnd, None);
        let _ = DestroyMenu(h);
        Cmd::from_id(id.0 as u32)
    };
    if let Some(c) = cmd {
        run(hwnd, c);
    }
}

/// Carry out `c` on the pin `hwnd`.
fn run(hwnd: HWND, c: Cmd) {
    match c {
        Cmd::Close => close(hwnd),
        Cmd::CloseAll => close_all(),
        Cmd::Copy | Cmd::Save | Cmd::SaveAs => {
            // Out of the borrow: Save As runs a modal dialog.
            let Some((img, at, centre, th)) = with_pin(hwnd, |p| (p.img.clone(), p.origin, p.centre(), p.th)) else { return };
            super::export(&img, at, centre, c, th);
        }
        Cmd::Zoom100 | Cmd::Opacity(_) => {
            with_pin(hwnd, |p| {
                if let Some(v) = apply(p.view, c) {
                    let about = p.centre();
                    p.set_view(v, about);
                }
            });
        }
    }
}

fn register() -> bool {
    static OK: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *OK.get_or_init(|| unsafe {
        let Ok(inst) = GetModuleHandleW(None) else { return false };
        let wc = WNDCLASSW {
            style: CS_DBLCLKS,
            lpfnWndProc: Some(wndproc),
            hInstance: inst.into(),
            lpszClassName: CLASS,
            hCursor: LoadCursorW(None, IDC_SIZEALL).unwrap_or_default(),
            ..Default::default()
        };
        RegisterClassW(&wc) != 0
    })
}

/// The UI scale (DPI times the UI size) of the monitor at `at`.
fn scale_at(at: (i32, i32)) -> f32 {
    unsafe {
        let mon = MonitorFromPoint(POINT { x: at.0, y: at.1 }, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(mon, &mut info);
        let (mut dx, mut dy) = (96u32, 96u32);
        if GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy).is_err() {
            dx = 96;
        }
        crate::uisize::apply((dx as f32 / 96.0).max(0.5))
    }
}

pub fn open(img: Arc<PixBuf>, at: (i32, i32), th: Theme) -> bool {
    if !register() {
        return false;
    }
    let ex = WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW;
    let Some(hwnd) = (unsafe {
        GetModuleHandleW(None)
            .ok()
            .and_then(|inst| CreateWindowExW(ex, CLASS, w!("Rustshot pin"), WS_POPUP, 0, 0, 1, 1, None, None, Some(inst.into()), None).ok())
    }) else {
        return false;
    };
    let (w, h) = img.dimensions();
    let s = scale_at((at.0 + w as i32 / 2, at.1 + h as i32 / 2));
    let mut pin = Pin { hwnd, img, th, s, view: View::default(), origin: at, dib: Dib::default(), px: Vec::new(), size: (0, 0), drag: None, wheel: 0 };
    pin.redraw();
    unsafe {
        // Tests never take the user's focus.
        #[cfg(not(test))]
        {
            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = SetForegroundWindow(hwnd);
        }
        #[cfg(test)]
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE);
    }
    PINS.with(|p| p.borrow_mut().push(pin));
    report();
    true
}

/// Close pin `hwnd` (out of the list first: destroying it runs its
/// window procedure).
fn close(hwnd: HWND) {
    let pin = PINS.with(|p| {
        let mut p = p.borrow_mut();
        let i = p.iter().position(|p| p.hwnd == hwnd)?;
        Some(p.remove(i))
    });
    if let Some(pin) = pin {
        unsafe {
            let _ = DestroyWindow(pin.hwnd);
        }
        drop(pin);
        report();
    }
}

pub fn close_all() {
    let pins = PINS.with(|p| std::mem::take(&mut *p.borrow_mut()));
    for pin in pins {
        unsafe {
            let _ = DestroyWindow(pin.hwnd);
        }
    }
    report();
}

/// The real windows (shown for a moment where the test puts them).
#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Foundation::RECT;

    fn rect(hwnd: HWND) -> RECT {
        let mut r = RECT::default();
        unsafe { GetWindowRect(hwnd, &mut r).unwrap() };
        r
    }

    #[test]
    fn a_pin_window_zooms_fades_moves_and_closes() {
        close_all();
        let img = Arc::new(crate::pin::tests::sample_shot(120, 80));
        assert!(open(img.clone(), (200, 150), crate::theme::DARK));
        assert!(open(img, (400, 150), crate::theme::LIGHT));
        assert_eq!(crate::pin::COUNT.load(std::sync::atomic::Ordering::SeqCst), 2);
        let hwnd = PINS.with(|p| p.borrow()[0].hwnd);
        unsafe {
            let ex = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
            for (f, name) in [(WS_EX_LAYERED, "layered"), (WS_EX_TOPMOST, "top-most"), (WS_EX_TOOLWINDOW, "no taskbar button")] {
                assert!(ex & f.0 != 0, "{name}");
            }
            assert_eq!(ex & WS_EX_NOACTIVATE.0, 0, "it takes the keyboard (Esc)");
            assert!(IsWindowVisible(hwnd).as_bool());
        }
        let fr = with_pin(hwnd, |p| Frame::at(p.s)).unwrap();
        let r = rect(hwnd);
        assert_eq!((r.left, r.top), fr.window_pos((200, 150)), "the image where it was captured");
        assert_eq!(((r.right - r.left) as u32, (r.bottom - r.top) as u32), fr.window((120, 80)));
        // Two notches up about the image's top-left corner: 121 %.
        wheel(hwnd, 120, false, (200, 150));
        wheel(hwnd, 60, false, (200, 150));
        wheel(hwnd, 60, false, (200, 150));
        assert_eq!(with_pin(hwnd, |p| p.view.zoom), Some(121));
        let r = rect(hwnd);
        assert_eq!(((r.right - r.left) as u32, (r.bottom - r.top) as u32), fr.window(scaled_size((120, 80), 121)));
        assert_eq!((r.left, r.top), fr.window_pos((200, 150)), "the corner under the pointer stays");
        // Ctrl+wheel: the opacity; the size stays.
        wheel(hwnd, -360, true, (0, 0));
        assert_eq!(with_pin(hwnd, |p| p.view), Some(View { zoom: 121, opacity: 70 }));
        // The menu's commands.
        run(hwnd, Cmd::Opacity(20));
        run(hwnd, Cmd::Zoom100);
        assert_eq!(with_pin(hwnd, |p| p.view), Some(View { zoom: 100, opacity: 20 }));
        // A drag moves it.
        with_pin(hwnd, |p| {
            p.origin = dragged(p.origin, (0, 0), (30, -20));
            p.place();
        });
        let o = with_pin(hwnd, |p| p.origin).unwrap();
        assert_eq!((rect(hwnd).left, rect(hwnd).top), fr.window_pos(o));
        run(hwnd, Cmd::Close);
        assert!(!unsafe { IsWindow(Some(hwnd)) }.as_bool());
        assert_eq!(crate::pin::COUNT.load(std::sync::atomic::Ordering::SeqCst), 1);
        let other = PINS.with(|p| p.borrow()[0].hwnd);
        run(other, Cmd::CloseAll);
        assert_eq!(crate::pin::COUNT.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert!(!unsafe { IsWindow(Some(other)) }.as_bool());
    }
}
