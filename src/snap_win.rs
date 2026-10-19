//! Window snapping on Windows: `EnumWindows` (front to back) with each
//! window's visible child windows (a `GetWindow` walk), in physical pixels
//! (the process is per-monitor DPI aware). What counts and which rect is
//! [`super::win_top_rect`]: the DWM frame as drawn (no invisible resize
//! borders or shadow), never a cloaked, minimised, click-through or own
//! window, nor the desktop.
//!
//! Cost: the cheap user32 reads come first; the two DWM queries (cloaked,
//! frame: a call into DWM each) only for windows that reach the shot and
//! could be cloaked or framed (a caption or sizing border, an app or
//! shell window): never for plain popups (menus, tooltips).
//! The window class (a kernel call) only for a window as big as the
//! desktop; parts by `GetWindow`, in process.

use super::{win_top_rect, Node, WinInfo, MAX_DEPTH, MAX_NODES, R};
use windows::core::BOOL;
use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT};
use windows::Win32::Graphics::Gdi::{ClientToScreen, GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST};
use windows::Win32::Graphics::Dwm::{
    DwmGetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS, DWMWA_VISIBLE_FRAME_BORDER_THICKNESS, DWMWA_WINDOW_CORNER_PREFERENCE,
    DWMWCP_DONOTROUND, DWMWCP_ROUND, DWMWCP_ROUNDSMALL, DWM_WINDOW_CORNER_PREFERENCE,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetClientRect, GetDesktopWindow, GetWindowDisplayAffinity, IsZoomed, WDA_NONE, GetLayeredWindowAttributes, GetShellWindow, GetWindow, GetWindowLongW, GetWindowRect, GetWindowThreadProcessId, IsIconic, IsWindowVisible, GWL_EXSTYLE, GWL_STYLE,
    LAYERED_WINDOW_ATTRIBUTES_FLAGS, LWA_ALPHA, WS_CAPTION, WS_EX_APPWINDOW, WS_EX_LAYERED, WS_EX_NOREDIRECTIONBITMAP, WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT,
    WS_THICKFRAME, WS_VISIBLE, GW_CHILD, GW_HWNDNEXT,
};

fn r_of(rc: RECT) -> R {
    R::new(rc.left, rc.top, rc.right, rc.bottom)
}

fn window_rect(h: HWND) -> Option<R> {
    let mut rc = RECT::default();
    unsafe { GetWindowRect(h, &mut rc) }.ok().map(|_| r_of(rc))
}

/// The client area of `h` in global px (`GetClientRect` from its origin).
fn client_rect(h: HWND) -> Option<R> {
    let mut rc = RECT::default();
    unsafe { GetClientRect(h, &mut rc) }.ok()?;
    let mut p = POINT::default();
    if !unsafe { ClientToScreen(h, &mut p) }.as_bool() {
        return None;
    }
    let r = R::new(p.x, p.y, p.x + rc.right, p.y + rc.bottom);
    (!r.is_empty()).then_some(r)
}

fn class_is(h: HWND, names: &[&str]) -> bool {
    let mut buf = [0u16; 64];
    let n = unsafe { GetClassNameW(h, &mut buf) };
    let n = n.clamp(0, buf.len() as i32) as usize;
    let s = String::from_utf16_lossy(&buf[..n]);
    names.iter().any(|c| *c == s)
}

/// What a top-level window is, for [`win_top_rect`]; `None` when a cheap
/// check already rules it out (hidden, ours, off the shot).
fn info(h: HWND, own_pid: Option<u32>, shell: (HWND, HWND), within: R) -> Option<WinInfo> {
    unsafe {
        if !IsWindowVisible(h).as_bool() || h == shell.0 || h == shell.1 {
            return None;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(h, Some(&mut pid));
        if own_pid == Some(pid) {
            return None;
        }
        let rect = window_rect(h)?;
        if rect.intersect(&within).is_empty() {
            return None;
        }
        let ex = GetWindowLongW(h, GWL_EXSTYLE) as u32;
        let style = GetWindowLongW(h, GWL_STYLE) as u32;
        let mut transparent = ex & WS_EX_TRANSPARENT.0 != 0;
        if !transparent && ex & WS_EX_LAYERED.0 != 0 {
            let (mut alpha, mut flags) = (255u8, LAYERED_WINDOW_ATTRIBUTES_FLAGS(0));
            if GetLayeredWindowAttributes(h, None, Some(&mut alpha), Some(&mut flags)).is_ok() {
                transparent = flags.0 & LWA_ALPHA.0 != 0 && alpha == 0;
            }
        }
        let iconic = IsIconic(h).as_bool();
        if transparent || iconic {
            return None;
        }
        let framed = style & (WS_CAPTION.0 | WS_THICKFRAME.0) != 0;
        // The desktop's wallpaper hosts cover the whole desktop; the class
        // (a kernel call) is asked only of a window that big.
        let shell = !framed && rect.intersect(&within) == within && class_is(h, &["Progman", "WorkerW"]);
        if shell {
            return None;
        }
        // DWM cloaks app windows (other virtual desktops, suspended or
        // closing UWP apps) and the shell's CoreWindows (Start, search);
        // a plain popup (menu, tooltip) is never asked: a DWM call each.
        let may_cloak = framed || ex & (WS_EX_NOREDIRECTIONBITMAP.0 | WS_EX_APPWINDOW.0) != 0;
        let mut c = 0u32;
        let cloaked =
            may_cloak && DwmGetWindowAttribute(h, DWMWA_CLOAKED, (&mut c as *mut u32).cast(), 4).is_ok() && c != 0;
        let frame = (framed && !cloaked)
            .then(|| {
                let mut f = RECT::default();
                DwmGetWindowAttribute(h, DWMWA_EXTENDED_FRAME_BOUNDS, (&mut f as *mut RECT).cast(), std::mem::size_of::<RECT>() as u32)
                    .ok()
                    .map(|_| r_of(f))
            })
            .flatten();
        Some(WinInfo {
            visible: true,
            iconic,
            cloaked,
            own: false,
            shell,
            tool: ex & WS_EX_TOOLWINDOW.0 != 0,
            transparent,
            rect,
            frame,
        })
    }
}

/// The visible child windows of `w` as a tree (front to back), at most
/// `budget` of them. Walked with `GetWindow` (child, then next sibling:
/// z-order, front first), which reads user32's shared window list in
/// process; `EnumChildWindows` lists the same windows but builds that list
/// in the kernel for every call (~25 µs a window).
fn kids_of(w: HWND, budget: &mut usize, depth: usize) -> Vec<Node> {
    let mut out = Vec::new();
    if depth > MAX_DEPTH {
        return out;
    }
    let mut c = unsafe { GetWindow(w, GW_CHILD) }.ok();
    while let Some(h) = c.filter(|h| !h.is_invalid()) {
        if *budget == 0 {
            break;
        }
        // A child is shown when it and its ancestors are; the ancestors
        // were, or the walk would not be here.
        let shown = unsafe { GetWindowLongW(h, GWL_STYLE) } as u32 & WS_VISIBLE.0 != 0;
        if shown && let Some(r) = window_rect(h).filter(|r| !r.is_empty()) {
            *budget -= 1;
            let kids = kids_of(h, budget, depth + 1);
            out.push(Node::new(r, kids));
        }
        c = unsafe { GetWindow(h, GW_HWNDNEXT) }.ok();
    }
    out
}

struct Tops {
    own_pid: Option<u32>,
    shell: (HWND, HWND),
    within: R,
    out: Vec<(HWND, R)>,
}

unsafe extern "system" fn top_cb(h: HWND, lp: LPARAM) -> BOOL {
    let t = unsafe { &mut *(lp.0 as *mut Tops) };
    if let Some(r) = info(h, t.own_pid, t.shell, t.within).as_ref().and_then(win_top_rect) {
        t.out.push((h, r));
    }
    BOOL(1)
}

/// Load and connect what the first snapshot would (dwmapi, its first
/// DWM query: ~25 ms once per process), off the capture's path.
pub fn warm() {
    let mut c = 0u32;
    let _ = unsafe { DwmGetWindowAttribute(GetDesktopWindow(), DWMWA_CLOAKED, (&mut c as *mut u32).cast(), 4) };
}

/// Every window that counts and reaches `within` (global px), front to
/// back, with its parts; `own_pid`'s windows left out.
pub fn windows(own_pid: Option<u32>, within: R) -> Option<Vec<Node>> {
    let shell = unsafe { (GetShellWindow(), GetDesktopWindow()) };
    let mut t = Tops { own_pid, shell, within, out: Vec::new() };
    unsafe { EnumWindows(Some(top_cb), LPARAM(&mut t as *mut Tops as isize)) }.ok()?;
    let mut left = MAX_NODES;
    Some(t.out.into_iter().map(|(h, r)| Node::new(r, kids_of(h, &mut left, 1)).with_client(client_rect(h))).collect())
}

/// A Rustshot window, of this process or another (its classes are
/// `rustshot_*`).
fn rustshot_class(h: HWND) -> bool {
    let mut buf = [0u16; 64];
    let n = unsafe { GetClassNameW(h, &mut buf) }.clamp(0, buf.len() as i32) as usize;
    String::from_utf16_lossy(&buf[..n]).starts_with("rustshot")
}

/// The radius of `h`'s rounded corners in px: Windows 11 rounds framed
/// windows that are not maximised (8 logical px) unless the window asked
/// otherwise (`DWMWA_WINDOW_CORNER_PREFERENCE`, when DWM tells); popups
/// stay square unless they asked. Windows 10: square.
fn corner_px(h: HWND, style: u32) -> f32 {
    unsafe {
        let mut thick = 0u32;
        // An attribute of Windows 11 (build 22000) DWM, readable there only.
        if DwmGetWindowAttribute(h, DWMWA_VISIBLE_FRAME_BORDER_THICKNESS, (&mut thick as *mut u32).cast(), 4).is_err() {
            return 0.0;
        }
        let mut pref = 0i32;
        let asked = DwmGetWindowAttribute(h, DWMWA_WINDOW_CORNER_PREFERENCE, (&mut pref as *mut i32).cast(), 4).is_ok();
        let framed = style & (WS_CAPTION.0 | WS_THICKFRAME.0) != 0;
        let logical = match (asked, DWM_WINDOW_CORNER_PREFERENCE(pref)) {
            (true, DWMWCP_DONOTROUND) => 0.0,
            (true, DWMWCP_ROUND) => 8.0,
            (true, DWMWCP_ROUNDSMALL) => 4.0,
            _ if framed && !IsZoomed(h).as_bool() => 8.0,
            _ => 0.0,
        };
        let dpi = GetDpiForWindow(h);
        logical * if dpi > 0 { dpi as f32 / 96.0 } else { 1.0 }
    }
}

/// `h`'s rect covers (nearly) all of its monitor.
fn covers_monitor(h: HWND, r: R) -> bool {
    unsafe {
        let mon = MonitorFromWindow(h, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        if !GetMonitorInfoW(mon, &mut mi).as_bool() {
            return false;
        }
        let m = r_of(mi.rcMonitor);
        let a = |r: R| i64::from(r.w()) * i64::from(r.h());
        a(r.intersect(&m)) * 100 >= a(m) * 95
    }
}

struct HiddenTops {
    own_pid: Option<u32>,
    shell: (HWND, HWND),
    within: R,
    out: Vec<crate::hidden::Top>,
}

/// One window for [`hidden_tops`]: hidden (`GetWindowDisplayAffinity`
/// other than `WDA_NONE`, any process may ask), or an ordinary window
/// that covers what is behind it.
fn hidden_top(h: HWND, t: &HiddenTops) -> Option<crate::hidden::Top> {
    // Every window that would snap counts (own captured ones too).
    let i = info(h, None, t.shell, t.within)?;
    let r = win_top_rect(&i)?;
    let mut aff = 0u32;
    if unsafe { GetWindowDisplayAffinity(h, &mut aff) }.is_err() || aff == WDA_NONE.0 {
        return Some(crate::hidden::Top { r, hidden: None });
    }
    // Hidden and ours (notices, the recording bar): not in the capture,
    // and nothing to explain.
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(h, Some(&mut pid)) };
    if t.own_pid == Some(pid) || rustshot_class(h) {
        return None;
    }
    unsafe {
        // `info` asks DWM only of framed and app windows.
        let mut c = 0u32;
        if DwmGetWindowAttribute(h, DWMWA_CLOAKED, (&mut c as *mut u32).cast(), 4).is_ok() && c != 0 {
            return None;
        }
        let mut f = RECT::default();
        let r = DwmGetWindowAttribute(h, DWMWA_EXTENDED_FRAME_BOUNDS, (&mut f as *mut RECT).cast(), std::mem::size_of::<RECT>() as u32)
            .ok()
            .map(|_| r_of(f))
            .filter(|f| !f.is_empty())
            .unwrap_or(r);
        // A layered window over a whole monitor is an overlay (the border
        // a meeting app draws round a shared screen), not a window.
        let ex = GetWindowLongW(h, GWL_EXSTYLE) as u32;
        if ex & WS_EX_LAYERED.0 != 0 && covers_monitor(h, r) {
            return None;
        }
        let style = GetWindowLongW(h, GWL_STYLE) as u32;
        Some(crate::hidden::Top { r, hidden: Some(corner_px(h, style)) })
    }
}

unsafe extern "system" fn hidden_cb(h: HWND, lp: LPARAM) -> BOOL {
    let t = unsafe { &mut *(lp.0 as *mut HiddenTops) };
    if let Some(top) = hidden_top(h, t) {
        t.out.push(top);
    }
    BOOL(1)
}

/// Every top-level window reaching `within`, front to back, those hidden
/// from capture marked (see [`super::hidden_tops`]).
pub fn hidden_tops(own_pid: Option<u32>, within: R) -> Option<Vec<crate::hidden::Top>> {
    let shell = unsafe { (GetShellWindow(), GetDesktopWindow()) };
    let mut t = HiddenTops { own_pid, shell, within, out: Vec::new() };
    unsafe { EnumWindows(Some(hidden_cb), LPARAM(&mut t as *mut HiddenTops as isize)) }.ok()?;
    Some(t.out)
}

#[cfg(test)]
mod tests {
    use super::super::Snapshot;
    use super::*;
    use windows::core::w;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, SetWindowPos, ShowWindow, HMENU, HWND_BOTTOM, SWP_NOACTIVATE, SWP_NOMOVE,
        SWP_NOSIZE, SW_SHOWNOACTIVATE, WS_CHILD, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_OVERLAPPEDWINDOW,
        WS_POPUP, WS_VISIBLE,
    };

    fn desktop() -> ((i32, i32), (u32, u32)) {
        let mons = crate::capture::monitors().expect("monitors");
        let o = (mons.iter().map(|m| m.x).min().unwrap(), mons.iter().map(|m| m.y).min().unwrap());
        let e = (mons.iter().map(|m| m.x + m.w as i32).max().unwrap(), mons.iter().map(|m| m.y + m.h as i32).max().unwrap());
        (o, ((e.0 - o.0) as u32, (e.1 - o.1) as u32))
    }

    fn all(o: (i32, i32), size: (u32, u32)) -> R {
        R::xywh(o.0, o.1, size.0 as i32, size.1 as i32)
    }

    /// A real window (titled, with DWM's invisible borders) with a button,
    /// and a borderless popup over it: the snapshot has the window's drawn
    /// frame, the button as its part (exact rect), the popup on its own,
    /// in front; our own windows are left out of a capture's view.
    #[test]
    #[ignore = "live display access (creates two test windows)"]
    fn live_snapshot_has_the_test_window_and_its_child() {
        crate::capture::enable_dpi_awareness();
        unsafe {
            let ex = WS_EX_TOPMOST | WS_EX_NOACTIVATE;
            let win = CreateWindowExW(ex, w!("STATIC"), w!("rustshot snap test"), WS_OVERLAPPEDWINDOW, 200, 150, 640, 420, None, None, None, None)
                .expect("window");
            let btn = CreateWindowExW(Default::default(), w!("BUTTON"), w!("part"), WS_CHILD | WS_VISIBLE, 30, 40, 160, 48, Some(win), Some(HMENU(7 as _)), None, None)
                .expect("button");
            let pop = CreateWindowExW(ex, w!("STATIC"), w!("rustshot snap popup"), WS_POPUP, 700, 250, 120, 90, None, None, None, None)
                .expect("popup");
            let _ = ShowWindow(win, SW_SHOWNOACTIVATE);
            let _ = ShowWindow(pop, SW_SHOWNOACTIVATE);
            std::thread::sleep(std::time::Duration::from_millis(300));
            let full = window_rect(win).unwrap();
            let mut fr = RECT::default();
            DwmGetWindowAttribute(win, DWMWA_EXTENDED_FRAME_BOUNDS, (&mut fr as *mut RECT).cast(), 16).expect("frame");
            let frame = r_of(fr);
            let part = window_rect(btn).unwrap();
            let popup = window_rect(pop).unwrap();
            eprintln!("window {full:?}, drawn frame {frame:?}, button {part:?}, popup {popup:?}");
            assert!(frame.w() < full.w(), "Windows 10/11: the frame is narrower than GetWindowRect");

            let (o, size) = desktop();
            // Our own windows are left out of a capture; here they are the test.
            let tops = windows(None, all(o, size)).expect("windows");
            let s = Snapshot::build(&tops, o, size);
            let img = |r: R| r.offset(-o.0, -o.1);
            let rects = s.window_rects();
            let i = rects.iter().position(|r| *r == img(frame)).expect("the test window, at its drawn frame");
            assert!(rects.iter().position(|r| *r == img(popup)).expect("the popup") < i, "the popup is in front");
            assert_eq!(s.part_rects(i), vec![img(part)], "the button is its only part");
            assert!(!rects.contains(&img(full)), "never the invisible borders");
            let (cx, cy) = (img(part).x0 + 5, img(part).y0 + 5);
            assert_eq!(s.chain(cx, cy), vec![img(frame), img(part)], "the button, one Tab into the window");
            // A capture's view (without this process's windows).
            let mine = windows(Some(std::process::id()), all(o, size)).expect("windows");
            let s2 = Snapshot::build(&mine, o, size);
            assert!(!s2.window_rects().contains(&img(frame)) && !s2.window_rects().contains(&img(popup)), "own windows are left out");
            let _ = DestroyWindow(pop);
            let _ = DestroyWindow(btn);
            let _ = DestroyWindow(win);
        }
    }

    /// The time a snapshot takes: this desktop, then with 300 more
    /// windows of 12 children each (at the bottom of the z-order, under
    /// everything else).
    #[test]
    #[ignore = "live display access (creates 3900 test windows for a moment)"]
    fn live_snapshot_time_with_many_windows() {
        crate::capture::enable_dpi_awareness();
        let (o, size) = desktop();
        let time = || {
            (0..5)
                .map(|_| {
                    let t = std::time::Instant::now();
                    let s = Snapshot::build(&windows(Some(std::process::id()), all(o, size)).unwrap(), o, size);
                    (t.elapsed().as_secs_f64() * 1000.0, s.windows(), s.parts())
                })
                .min_by(|a, b| a.0.total_cmp(&b.0))
                .unwrap()
        };
        let (ms, w, p) = time();
        eprintln!("this desktop: best {ms:.2} ms, {w} windows, {p} parts");
        let mut made = Vec::new();
        unsafe {
            for i in 0..300 {
                let ex = WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW;
                let (x, y) = (o.0 + (i % 20) * 60, o.1 + (i / 20) * 40);
                let h = CreateWindowExW(ex, w!("STATIC"), w!("rustshot snap load"), WS_POPUP, x, y, 280, 180, None, None, None, None)
                    .expect("load window");
                for j in 0..12 {
                    let _ = CreateWindowExW(Default::default(), w!("STATIC"), w!(""), WS_CHILD | WS_VISIBLE, (j % 4) * 70, (j / 4) * 60, 60, 50, Some(h), None, None, None);
                }
                let _ = SetWindowPos(h, Some(HWND_BOTTOM), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
                let _ = ShowWindow(h, SW_SHOWNOACTIVATE);
                made.push(h);
            }
            // Counted as the capture would (other processes' windows): ours here.
            let t = std::time::Instant::now();
            let s = Snapshot::build(&windows(None, all(o, size)).unwrap(), o, size);
            let ms = t.elapsed().as_secs_f64() * 1000.0;
            let best = (0..4)
                .map(|_| {
                    let t = std::time::Instant::now();
                    std::hint::black_box(Snapshot::build(&windows(None, all(o, size)).unwrap(), o, size));
                    t.elapsed().as_secs_f64() * 1000.0
                })
                .fold(ms, f64::min);
            for h in made {
                let _ = DestroyWindow(h);
            }
            eprintln!("with 300 more windows: first {ms:.2} ms, best {best:.2} ms, {} windows, {} parts", s.windows(), s.parts());
            assert!(s.windows() >= 300 && s.parts() >= 3600, "all listed");
        }
    }
}
