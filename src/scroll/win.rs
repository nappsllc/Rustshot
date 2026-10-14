//! Windows: scroll the window under the region. `WM_MOUSEWHEEL` is
//! posted to that window (what Windows itself does for "scroll inactive
//! windows"); when that moves nothing, UI Automation's ScrollPattern of
//! the element there (or its nearest scrollable ancestor) scrolls it by
//! lines. No input is synthesised (no `SendInput`): only the target
//! window gets the messages.

use super::{text, Interrupted, Wheel};
use anyhow::{bail, Result};
use windows::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationScrollPattern, ScrollAmount_NoAmount, ScrollAmount_SmallIncrement,
    UIA_ScrollPatternId,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_ESCAPE};
use windows::Win32::UI::WindowsAndMessaging::{
    GetAncestor, GetWindowThreadProcessId, IsWindow, PostMessageW, WindowFromPoint, GA_ROOT, WM_MOUSEWHEEL,
};

/// One wheel notch.
const NOTCH: i32 = 120;

/// The scroll pattern, made and used on the capture's worker thread only.
struct Pattern(IUIAutomationScrollPattern);

// SAFETY: created by `fallback` on the worker thread (the MTA) and used
// only there; the wheel is moved to that thread before either happens.
unsafe impl Send for Pattern {}

pub struct WinWheel {
    /// The window under the region's centre (a raw handle: `HWND` is not
    /// `Send`).
    hwnd: isize,
    at: (i32, i32),
    pattern: Option<Pattern>,
}

impl WinWheel {
    /// The window under `at` (screen px); not one of ours.
    #[cfg_attr(test, allow(dead_code))]
    pub fn at(at: (i32, i32)) -> Result<WinWheel> {
        WinWheel::find(at, false)
    }

    /// [`WinWheel::at`]; `own`: one of ours will do (the live test's).
    fn find(at: (i32, i32), own: bool) -> Result<WinWheel> {
        let hwnd = unsafe { WindowFromPoint(POINT { x: at.0, y: at.1 }) };
        if hwnd.is_invalid() {
            bail!(text::NO_TARGET);
        }
        let mut pid = 0;
        unsafe { GetWindowThreadProcessId(GetAncestor(hwnd, GA_ROOT), Some(&mut pid)) };
        if pid == unsafe { GetCurrentProcessId() } && !own {
            bail!(text::NO_TARGET);
        }
        Ok(WinWheel { hwnd: hwnd.0 as isize, at, pattern: None })
    }

    fn hwnd(&self) -> HWND {
        HWND(self.hwnd as *mut _)
    }
}

/// `WM_MOUSEWHEEL`'s parameters: the delta (negative: down) and the
/// pointer's screen position.
pub(crate) fn wheel_params(delta: i32, at: (i32, i32)) -> (WPARAM, LPARAM) {
    let wp = ((delta as i16 as u16 as usize) << 16) as isize as usize;
    let lp = ((at.1 as i16 as u16 as isize) << 16) | (at.0 as i16 as u16 as isize);
    (WPARAM(wp), LPARAM(lp))
}

impl Wheel for WinWheel {
    fn scroll(&mut self, notches: u32) -> Result<()> {
        if !unsafe { IsWindow(Some(self.hwnd())) }.as_bool() {
            bail!("the window under the selection closed");
        }
        // Each step: the same window is still on top there (another one
        // coming over would be stitched in).
        // SAFETY: plain window queries.
        let same = unsafe {
            let now = WindowFromPoint(POINT { x: self.at.0, y: self.at.1 });
            !now.is_invalid() && GetAncestor(now, GA_ROOT) == GetAncestor(self.hwnd(), GA_ROOT)
        };
        if !same {
            return Err(Interrupted(text::CHANGED.get()).into());
        }
        if let Some(p) = &self.pattern {
            for _ in 0..notches {
                unsafe { p.0.Scroll(ScrollAmount_NoAmount, ScrollAmount_SmallIncrement)? };
            }
            return Ok(());
        }
        let (wp, lp) = wheel_params(-NOTCH, self.at);
        for _ in 0..notches {
            if let Err(e) = unsafe { PostMessageW(Some(self.hwnd()), WM_MOUSEWHEEL, wp, lp) } {
                // UIPI: a window of an elevated app takes no messages from us.
                if e.code() == windows::Win32::Foundation::E_ACCESSDENIED {
                    bail!(text::ELEVATED);
                }
                return Err(e.into());
            }
        }
        Ok(())
    }

    /// The wheel did nothing: the element under the point (or its nearest
    /// vertically scrollable ancestor) through UI Automation.
    fn fallback(&mut self) -> bool {
        if self.pattern.is_some() {
            return false;
        }
        crate::rec::win::com_init();
        let found = (|| -> windows::core::Result<Option<IUIAutomationScrollPattern>> {
            let uia: IUIAutomation = unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)? };
            let walker = unsafe { uia.ControlViewWalker()? };
            let mut el = unsafe { uia.ElementFromPoint(POINT { x: self.at.0, y: self.at.1 })? };
            let me = unsafe { GetCurrentProcessId() } as i32;
            for _ in 0..16 {
                if unsafe { el.CurrentProcessId() }.unwrap_or(me) == me {
                    return Ok(None);
                }
                if let Ok(p) = unsafe { el.GetCurrentPatternAs::<IUIAutomationScrollPattern>(UIA_ScrollPatternId) }
                    && unsafe { p.CurrentVerticallyScrollable() }.is_ok_and(|b| b.as_bool())
                {
                    return Ok(Some(p));
                }
                match unsafe { walker.GetParentElement(&el) } {
                    Ok(parent) => el = parent,
                    Err(_) => break,
                }
            }
            Ok(None)
        })();
        match found {
            Ok(Some(p)) => {
                self.pattern = Some(Pattern(p));
                true
            }
            _ => false,
        }
    }
}

/// Esc held now (the keyboard's state; no hook).
#[cfg_attr(test, allow(dead_code))]
pub fn esc_down() -> bool {
    unsafe { GetAsyncKeyState(VK_ESCAPE.0 as i32) as u16 & 0x8000 != 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wheel_parameters_pack_delta_and_point() {
        let (wp, lp) = wheel_params(-120, (100, 200));
        assert_eq!(wp.0 >> 16 & 0xFFFF, 0xFF88, "-120 in the high word");
        assert_eq!(wp.0 & 0xFFFF, 0, "no keys held");
        assert_eq!(lp.0 & 0xFFFF, 100);
        assert_eq!(lp.0 >> 16 & 0xFFFF, 200);
        // A monitor left of the primary: negative x.
        let (_, lp) = wheel_params(-120, (-50, 10));
        assert_eq!(lp.0 & 0xFFFF, 0xFFCE);
    }

    /// Live: a window of our own with tall content that scrolls on
    /// `WM_MOUSEWHEEL` is captured whole (no other window is touched).
    /// `cargo test live_scroll_capture -- --ignored --nocapture`
    #[test]
    #[ignore = "live: opens a window and captures the screen"]
    fn live_scroll_capture() {
        super::live::go();
    }
}

/// The live test's own scrollable window.
#[cfg(test)]
mod live {
    use super::super::{run, Ctl, Pace};
    use std::sync::atomic::{AtomicI32, Ordering};
    use windows::core::w;
    use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
    use windows::Win32::Graphics::Gdi::{BeginPaint, EndPaint, FillRect, CreateSolidBrush, DeleteObject, HGDIOBJ, InvalidateRect, PAINTSTRUCT};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::*;

    /// Content height and the offset now.
    const CONTENT: i32 = 2400;
    static OFF: AtomicI32 = AtomicI32::new(0);

    unsafe extern "system" fn proc_(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        unsafe {
            match msg {
                WM_MOUSEWHEEL => {
                    let d = (wp.0 >> 16) as u16 as i16 as i32;
                    let mut rc = RECT::default();
                    let _ = GetClientRect(hwnd, &mut rc);
                    let max = CONTENT - rc.bottom;
                    let o = (OFF.load(Ordering::SeqCst) - d / 3).clamp(0, max.max(0));
                    OFF.store(o, Ordering::SeqCst);
                    let _ = InvalidateRect(Some(hwnd), None, false);
                    LRESULT(0)
                }
                WM_PAINT => {
                    let mut ps = PAINTSTRUCT::default();
                    let dc = BeginPaint(hwnd, &mut ps);
                    let mut rc = RECT::default();
                    let _ = GetClientRect(hwnd, &mut rc);
                    let o = OFF.load(Ordering::SeqCst);
                    // Bands of 24 px, each its own color, with a dark mark.
                    for y in 0..rc.bottom {
                        let cy = y + o;
                        let band = cy / 24;
                        let c = COLORREF(((band * 53 % 200) as u32) | (((band * 97 % 220) as u32) << 8) | (((cy % 24) as u32 * 8) << 16));
                        let br = CreateSolidBrush(c);
                        let x1 = 40 + (band * 37 % 300);
                        let line = RECT { left: 0, top: y, right: x1, bottom: y + 1 };
                        FillRect(dc, &line, br);
                        let _ = DeleteObject(HGDIOBJ(br.0));
                        let rest = RECT { left: x1, top: y, right: rc.right, bottom: y + 1 };
                        let wb = CreateSolidBrush(COLORREF(0x00FFFFFF));
                        FillRect(dc, &rest, wb);
                        let _ = DeleteObject(HGDIOBJ(wb.0));
                    }
                    let _ = EndPaint(hwnd, &ps);
                    LRESULT(0)
                }
                _ => DefWindowProcW(hwnd, msg, wp, lp),
            }
        }
    }

    pub fn go() {
        let _guard = crate::wind::test_window_lock();
        crate::capture::enable_dpi_awareness();
        unsafe {
            let inst = GetModuleHandleW(None).unwrap();
            let wc = WNDCLASSW { lpfnWndProc: Some(proc_), hInstance: inst.into(), lpszClassName: w!("rustshot_scroll_test"), ..Default::default() };
            RegisterClassW(&wc);
            let hwnd = CreateWindowExW(
                WS_EX_TOPMOST,
                w!("rustshot_scroll_test"),
                w!("Rustshot scroll test"),
                WS_POPUP | WS_VISIBLE,
                100,
                100,
                400,
                300,
                None,
                None,
                Some(inst.into()),
                None,
            )
            .unwrap();
            let pump = || {
                let mut m = MSG::default();
                while PeekMessageW(&mut m, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&m);
                    DispatchMessageW(&m);
                }
            };
            for _ in 0..20 {
                pump();
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            let mut rc = RECT::default();
            let _ = GetWindowRect(hwnd, &mut rc);
            let area = (rc.left, rc.top, (rc.right - rc.left) as u32, (rc.bottom - rc.top) as u32);
            let mut wheel = super::WinWheel::find((area.0 + 200, area.1 + 150), true).expect("our test window");
            assert_eq!(wheel.hwnd, hwnd.0 as isize, "the wheel goes to the test window");
            let ctl = Ctl::default();
            let mut grab = || {
                // The window paints on this thread between grabs.
                for _ in 0..3 {
                    pump();
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                super::super::grab_area(area)
            };
            let t = std::time::Instant::now();
            let (pic, end) = run(&mut grab, &mut wheel, &ctl, Pace::default(), &|| false).expect("run");
            println!("scrolled {:?}: {}x{} in {:?}", end, pic.width(), pic.height(), t.elapsed());
            let _ = DestroyWindow(hwnd);
            assert_eq!(end, super::super::End::Bottom);
            assert_eq!(pic.height() as i32, CONTENT, "the whole content");
        }
    }
}
