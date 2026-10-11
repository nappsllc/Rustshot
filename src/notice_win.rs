//! Windows: the notice in a layered, top-most tool window that never
//! activates, lets clicks through (`WS_EX_TRANSPARENT`) and is excluded
//! from capture. It lives on the UI thread and fades on its own timer.

use super::{opacity, place, render, size, Note, FADE_IN};
use crate::rec_ui::Dib;
use crate::theme::Theme;
use std::cell::RefCell;
use std::time::Instant;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::*;

const CLASS: PCWSTR = w!("rustshot_notice");
const TIMER: usize = 1;
const TICK_MS: u32 = 16;

struct Up {
    note: Note,
    th: Theme,
    s: f32,
    pos: (i32, i32),
    since: Instant,
    /// Opacity of the frame on screen (None: none drawn yet).
    k: Option<f32>,
}

struct Win {
    hwnd: HWND,
    dib: Dib,
    up: Option<Up>,
}

thread_local! {
    static WIN: RefCell<Option<Win>> = const { RefCell::new(None) };
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_TIMER => {
                tick();
                LRESULT(0)
            }
            WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
            WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

fn create() -> Option<HWND> {
    static CLASS_OK: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let ok = *CLASS_OK.get_or_init(|| unsafe {
        let Ok(inst) = GetModuleHandleW(None) else { return false };
        let wc = WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: inst.into(), lpszClassName: CLASS, ..Default::default() };
        RegisterClassW(&wc) != 0
    });
    if !ok {
        return None;
    }
    let ex = WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE;
    let hwnd = unsafe {
        let inst = GetModuleHandleW(None).ok()?;
        CreateWindowExW(ex, CLASS, w!("Rustshot"), WS_POPUP, 0, 0, 1, 1, None, None, Some(inst.into()), None).ok()?
    };
    // Never in a screenshot or a recording; Windows 10 2004+.
    if let Err(e) = unsafe { SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE) } {
        eprintln!("rustshot: cannot exclude the notice from capture: {e}");
    }
    Some(hwnd)
}

/// The work area (physical px) and UI scale (its DPI times the UI size)
/// of the monitor nearest `at`.
fn monitor_at(at: (i32, i32)) -> (crate::capture::IRect, f32) {
    unsafe {
        let mon = MonitorFromPoint(POINT { x: at.0, y: at.1 }, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(mon, &mut info);
        let r = info.rcWork;
        let (mut dx, mut dy) = (96u32, 96u32);
        if GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy).is_err() {
            dx = 96;
        }
        ((r.left, r.top, (r.right - r.left).max(0) as u32, (r.bottom - r.top).max(0) as u32), crate::uisize::apply((dx as f32 / 96.0).max(0.5)))
    }
}

pub fn show(note: Note, at: (i32, i32), th: Theme) -> bool {
    let (work, s) = monitor_at(at);
    let pos = place(work, size(&note.text, s));
    let shown = WIN.with(|w| {
        let mut w = w.borrow_mut();
        if w.is_none() {
            *w = create().map(|hwnd| Win { hwnd, dib: Dib::default(), up: None });
        }
        let Some(win) = w.as_mut() else { return false };
        win.up = Some(Up { note, th, s, pos, since: Instant::now(), k: None });
        unsafe {
            let _ = SetTimer(Some(win.hwnd), TIMER, TICK_MS, None);
        }
        true
    });
    if shown {
        tick();
    }
    shown
}

pub fn replace(note: Note) -> bool {
    let swapped = WIN.with(|w| {
        let mut w = w.borrow_mut();
        let Some(win) = w.as_mut() else { return false };
        let Some(up) = win.up.as_mut() else { return false };
        let (work, s) = monitor_at(up.pos);
        let now = Instant::now();
        up.pos = place(work, size(&note.text, s));
        (up.note, up.s, up.k) = (note, s, None);
        // On screen already: no second fade-in.
        up.since = now.checked_sub(FADE_IN).unwrap_or(now);
        true
    });
    if swapped {
        tick();
    }
    swapped
}

pub fn hide() {
    WIN.with(|w| {
        if let Some(win) = w.borrow_mut().as_mut() {
            take_down(win);
        }
    });
}

fn take_down(win: &mut Win) {
    win.up = None;
    unsafe {
        let _ = KillTimer(Some(win.hwnd), TIMER);
        let _ = ShowWindow(win.hwnd, SW_HIDE);
    }
}

/// Draw the next frame of the fade, or take the note down once it is over.
fn tick() {
    WIN.with(|w| {
        let mut w = w.borrow_mut();
        let Some(win) = w.as_mut() else { return };
        let Some(up) = win.up.as_mut() else { return };
        let Some(k) = opacity(up.since.elapsed(), up.note.ttl) else {
            take_down(win);
            return;
        };
        if up.k.is_some_and(|old| (old - k).abs() < 0.004) {
            return; // holding: no new frame
        }
        let first = up.k.is_none();
        up.k = Some(k);
        let (bw, bh, px) = render(&up.note, &up.th, up.s, k);
        win.dib.put(win.hwnd, up.pos.0, up.pos.1, bw, bh, &px);
        if first {
            unsafe {
                let _ = ShowWindow(win.hwnd, SW_SHOWNOACTIVATE);
                let _ = SetWindowPos(win.hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
            }
        }
    });
}

/// The real window (shown for a moment at the bottom of the screen).
#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::style::ToastKind;

    #[test]
    fn the_notice_window_never_takes_focus_or_clicks_and_is_not_captured() {
        let note = |t: &str| Note { text: t.into(), kind: ToastKind::Info, ttl: std::time::Duration::from_millis(300) };
        assert!(!replace(note("x")), "nothing up yet");
        assert!(show(note("Uploading\u{2026}"), (0, 0), crate::theme::DARK));
        let hwnd = WIN.with(|w| w.borrow().as_ref().unwrap().hwnd);
        unsafe {
            let ex = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
            for (f, name) in [(WS_EX_LAYERED, "layered"), (WS_EX_TRANSPARENT, "click-through"), (WS_EX_NOACTIVATE, "no-activate"), (WS_EX_TOPMOST, "top-most"), (WS_EX_TOOLWINDOW, "no taskbar button")] {
                assert!(ex & f.0 != 0, "{name}");
            }
            let mut aff = 0u32;
            GetWindowDisplayAffinity(hwnd, &mut aff).unwrap();
            assert_eq!(aff, WDA_EXCLUDEFROMCAPTURE.0, "excluded from capture");
            assert!(IsWindowVisible(hwnd).as_bool());
            assert_ne!(windows::Win32::UI::Input::KeyboardAndMouse::GetFocus(), hwnd);
        }
        assert!(replace(note("Link copied \u{b7} i.imgur.com/x.png")));
        let shown = WIN.with(|w| w.borrow().as_ref().unwrap().up.as_ref().map(|u| u.note.text.clone()));
        assert_eq!(shown.as_deref(), Some("Link copied \u{b7} i.imgur.com/x.png"));
        hide();
        assert!(!unsafe { IsWindowVisible(hwnd) }.as_bool());
        assert!(!replace(note("x")), "gone");
    }
}
