//! Win32 window implementation: the overlay and the decorated `run_window`
//! (class registration, message pumps, input event delivery) and
//! framebuffer presentation (StretchDIBits).

use super::*;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{
    GetLastError, ERROR_CLASS_ALREADY_EXISTS, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CombineRgn, CreateRectRgn, DeleteObject, EndPaint, GetMonitorInfoW, GetRegionData,
    GetUpdateRgn, InvalidateRect, InvalidateRgn, MonitorFromPoint, SetRectRgn, ScreenToClient, StretchDIBits, UpdateWindow,
    BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    PAINTSTRUCT, RGNDATA, RGNDATAHEADER, RGN_OR, SRCCOPY,
};
use windows::Win32::UI::HiDpi::{
    AdjustWindowRectExForDpi, GetDpiForMonitor, GetDpiForWindow, SetThreadDpiAwarenessContext,
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, MDT_EFFECTIVE_DPI,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture};
use windows::Win32::UI::WindowsAndMessaging::*;

fn cursor_id(c: Cursor) -> PCWSTR {
    match c {
        Cursor::Arrow => IDC_ARROW,
        Cursor::Cross => IDC_CROSS,
        Cursor::IBeam => IDC_IBEAM,
        Cursor::SizeNS => IDC_SIZENS,
        Cursor::SizeWE => IDC_SIZEWE,
        Cursor::SizeNWSE => IDC_SIZENWSE,
        Cursor::SizeNESW => IDC_SIZENESW,
        Cursor::Move => IDC_SIZEALL,
    }
}

static CLASS_REGISTERED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

unsafe fn module_handle() -> windows::core::Result<*mut core::ffi::c_void> {
    unsafe {
        GetModuleHandleW(PCWSTR(core::ptr::null()))
            .map(|h| h.0)
    }
}

/// RegisterClassExW succeeded, or the class exists already (another thread
/// won the race): either way the class is usable.
unsafe fn class_ok(atom: u16) -> bool {
    atom != 0 || unsafe { GetLastError() } == ERROR_CLASS_ALREADY_EXISTS
}

unsafe fn register_class() {
    // Only a success is remembered: a failed registration is retried by
    // the next call.
    if CLASS_REGISTERED.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    unsafe {
        let h = module_handle().unwrap_or(core::ptr::null_mut());
        let mut wc: WNDCLASSEXW = std::mem::zeroed();
        wc.cbSize = std::mem::size_of::<WNDCLASSEXW>() as u32;
        wc.style = CS_HREDRAW | CS_VREDRAW;
        wc.lpfnWndProc = Some(wndproc);
        wc.hInstance = HINSTANCE(h);
        wc.hCursor = LoadCursorW(None, IDC_ARROW).unwrap_or_default();
        wc.lpszClassName = w!("rustshot_overlay");
        if class_ok(RegisterClassExW(&wc)) {
            CLASS_REGISTERED.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }
}

unsafe fn x_of(lp: LPARAM) -> i32 {
    (lp.0 & 0xffff) as u16 as i16 as i32
}

unsafe fn y_of(lp: LPARAM) -> i32 {
    ((lp.0 >> 16) & 0xffff) as u16 as i16 as i32
}

/// Ask for a repaint of the window's client area.
pub fn invalidate(hwnd: HWND) {
    // A null handle would invalidate every window on the desktop.
    if hwnd.is_invalid() {
        return;
    }
    crate::perf::invalidated();
    unsafe {
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}

/// Invalidate rects `[x0, y0, x1, y1]` of the client area (no erase).
///
/// One InvalidateRgn of their union: a single InvalidateRect took 30 to
/// 400 us on a 5120x1440 overlay, so a dozen per mouse move at 1 kHz kept
/// the thread busy with moves (four per paint) and starved WM_PAINT.
pub fn invalidate_rects(hwnd: HWND, rects: &[[i32; 4]]) {
    if hwnd.is_invalid() || rects.is_empty() {
        return;
    }
    let _p = crate::perf::span(crate::perf::K::Inval);
    crate::perf::invalidated();
    let rc = |r: &[i32; 4]| RECT { left: r[0], top: r[1], right: r[2], bottom: r[3] };
    unsafe {
        if let [r] = rects {
            let _ = InvalidateRect(Some(hwnd), Some(&rc(r)), false);
            return;
        }
        let all = CreateRectRgn(rects[0][0], rects[0][1], rects[0][2], rects[0][3]);
        let one = CreateRectRgn(0, 0, 0, 0);
        if all.is_invalid() || one.is_invalid() {
            // Out of GDI handles: rect by rect.
            for r in rects {
                let _ = InvalidateRect(Some(hwnd), Some(&rc(r)), false);
            }
        } else {
            for r in &rects[1..] {
                let _ = SetRectRgn(one, r[0], r[1], r[2], r[3]);
                CombineRgn(Some(all), Some(all), Some(one), RGN_OR);
            }
            let _ = InvalidateRgn(hwnd, Some(all), false);
        }
        for g in [one, all] {
            if !g.is_invalid() {
                let _ = DeleteObject(g.into());
            }
        }
    }
}

/// The update region as rects (before `BeginPaint` validates it). A
/// region of many pieces (a drag's bands: the crosshair's full-height
/// line crosses every one) is coalesced into a few rects covering it,
/// never its bounding box (that would repaint the whole overlay).
unsafe fn update_rects(hwnd: HWND) -> Vec<[i32; 4]> {
    unsafe {
        let rgn = CreateRectRgn(0, 0, 0, 0);
        // 0 = ERROR, 1 = NULLREGION: nothing to read.
        let out = if GetUpdateRgn(hwnd, rgn, false).0 > 1 { region_rects(rgn) } else { Vec::new() };
        let _ = DeleteObject(rgn.into());
        coalesce(out, MAX_PAINT_RECTS)
    }
}

/// Tests and benches: `rects` invalidated as one region (as
/// [`invalidate_rects`] does) and read back as a paint would get them.
#[cfg(test)]
pub fn paint_rects_for(rects: &[[i32; 4]]) -> Vec<[i32; 4]> {
    if rects.is_empty() {
        return Vec::new();
    }
    unsafe {
        let all = CreateRectRgn(rects[0][0], rects[0][1], rects[0][2], rects[0][3]);
        let one = CreateRectRgn(0, 0, 0, 0);
        for r in &rects[1..] {
            let _ = SetRectRgn(one, r[0], r[1], r[2], r[3]);
            CombineRgn(Some(all), Some(all), Some(one), RGN_OR);
        }
        let out = region_rects(all);
        let _ = DeleteObject(one.into());
        let _ = DeleteObject(all.into());
        coalesce(out, MAX_PAINT_RECTS)
    }
}

/// The most rects a paint takes (more are coalesced).
const MAX_PAINT_RECTS: usize = 32;

/// The rects of region `rgn` (Windows' y-banded form).
unsafe fn region_rects(rgn: windows::Win32::Graphics::Gdi::HRGN) -> Vec<[i32; 4]> {
    unsafe {
        let mut out = Vec::new();
        let n = GetRegionData(rgn, 0, None);
        if n as usize >= std::mem::size_of::<RGNDATAHEADER>() {
            // u64 storage keeps the RECTs after the header aligned.
            let mut buf = vec![0u64; (n as usize).div_ceil(8)];
            let data = buf.as_mut_ptr() as *mut RGNDATA;
            if GetRegionData(rgn, n, Some(data)) != 0 {
                let hdr = &(*data).rdh;
                let rects = std::slice::from_raw_parts((*data).Buffer.as_ptr() as *const RECT, hdr.nCount as usize);
                out.extend(rects.iter().map(|r| [r.left, r.top, r.right, r.bottom]));
            }
        }
        out
    }
}

/// `rects` (disjoint, y-banded) as at most `max` rects covering them:
/// first the bands of one column joined top to bottom (same left and
/// right, touching), then the pair whose union adds the least area merged,
/// until few enough are left.
fn coalesce(rects: Vec<[i32; 4]>, max: usize) -> Vec<[i32; 4]> {
    if rects.len() <= max {
        return rects;
    }
    let mut v: Vec<[i32; 4]> = Vec::with_capacity(rects.len());
    let mut sorted = rects;
    crate::sort::by_key(&mut sorted, |r| (r[0], r[2], r[1]));
    for r in sorted {
        match v.last_mut() {
            Some(l) if l[0] == r[0] && l[2] == r[2] && l[3] == r[1] => l[3] = r[3],
            _ => v.push(r),
        }
    }
    let area = |r: &[i32; 4]| (r[2] - r[0]) as i64 * (r[3] - r[1]) as i64;
    let union = |a: &[i32; 4], b: &[i32; 4]| [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])];
    while v.len() > max {
        let mut best = (i64::MAX, 0, 1);
        for i in 0..v.len() {
            for j in i + 1..v.len() {
                let cost = area(&union(&v[i], &v[j])) - area(&v[i]) - area(&v[j]);
                if cost < best.0 {
                    best = (cost, i, j);
                }
            }
        }
        let (_, i, j) = best;
        let u = union(&v[i], &v[j]);
        v.swap_remove(j);
        v[i] = u;
    }
    v
}

// --- the overlay's pointer moves: one frame per display refresh ---------
//
// A mouse reporting at 1-8 kHz sends a move far more often than the
// display shows a frame (60-240 Hz), and input outranks WM_PAINT in the
// queue: handled (and painted) one by one, the moves kept the thread busy
// and the paint waited behind them, so the loupe and crosshair trailed the
// pointer. The overlay's WM_MOUSEMOVE now only notes where the pointer
// is; the loop draws that, once per refresh, late in it (the freshest
// position, done before the compositor takes the frame). Any other input
// (a press, a key) first takes the noted move, so its order is kept.

/// Sent to the overlay by its loop: handle the noted move and paint now.
const WM_APP_MOVE: u32 = WM_APP + 0x31;

/// The overlay's noted move: its window, where, and the times of the
/// oldest and newest move behind it (`perf`).
#[derive(Clone, Copy)]
struct Noted {
    hwnd: isize,
    x: i32,
    y: i32,
    times: Option<(std::time::Instant, std::time::Instant)>,
}

thread_local! {
    static NOTED: std::cell::Cell<Option<Noted>> = const { std::cell::Cell::new(None) };
    static PACER: std::cell::RefCell<Pacer> = const { std::cell::RefCell::new(Pacer::new()) };
}

/// Note a move to `(x, y)` (at `t`, see `perf::move_time`).
fn note_move(hwnd: HWND, x: i32, y: i32, t: Option<std::time::Instant>) {
    let prev = NOTED.get().filter(|n| n.hwnd == hwnd.0 as isize);
    let times = match (prev.and_then(|p| p.times), t) {
        (Some((first, _)), Some(t)) => Some((first, t)),
        (None, Some(t)) => Some((t, t)),
        (old, None) => old,
    };
    NOTED.set(Some(Noted { hwnd: hwnd.0 as isize, x, y, times }));
}

/// Hand the noted move of `hwnd` (if any) to the driver; true when there
/// was one.
unsafe fn take_move(hwnd: HWND, drv: &mut dyn Driver) -> bool {
    let Some(n) = NOTED.get().filter(|n| n.hwnd == hwnd.0 as isize) else { return false };
    NOTED.set(None);
    let _m = crate::perf::mouse_move(n.times);
    drv.on_event(Ev::Move { x: n.x, y: n.y });
    true
}

/// A move is noted (for any overlay on this thread).
fn move_noted() -> bool {
    NOTED.get().is_some()
}

/// Messages that must see the noted move first (they act where the
/// pointer is, or change what it is over).
fn takes_move_first(msg: u32) -> bool {
    matches!(
        msg,
        WM_LBUTTONDOWN
            | WM_LBUTTONUP
            | WM_MBUTTONDOWN
            | WM_MBUTTONUP
            | WM_RBUTTONDOWN
            | WM_RBUTTONUP
            | WM_MOUSEWHEEL
            | WM_KEYDOWN
            | WM_KEYUP
            | WM_CHAR
    )
}

/// The cursor the driver wants now, set when the pointer is over `hwnd`
/// (or it has the capture): after a deferred move nothing else asks
/// (WM_SETCURSOR came with the move, before it was handled).
unsafe fn refresh_cursor(hwnd: HWND, drv: &dyn Driver) {
    unsafe {
        let mut p = POINT::default();
        let over = windows::Win32::UI::Input::KeyboardAndMouse::GetCapture() == hwnd
            || (GetCursorPos(&mut p).is_ok() && WindowFromPoint(p) == hwnd);
        if over && let Ok(c) = LoadCursorW(None, cursor_id(drv.cursor())) {
            let _ = SetCursor(Some(c));
        }
    }
}

/// When to draw the noted move: once per display refresh, `lead` ms
/// before the vblank (what the last frames took, plus a margin), so the
/// frame is done in time and shows the freshest position.
pub(crate) struct Pacer {
    /// The vblank the last frame was made for (ms, QPC clock).
    made_for: Option<f64>,
    /// What a frame takes (ms, smoothed).
    cost: f64,
    /// The display's timing: the last vblank (ms) and the period (ms),
    /// and when they were read (ms).
    timing: Option<(f64, f64, f64)>,
}

/// Ready this long before the vblank besides the frame's cost (ms).
const PACE_MARGIN_MS: f64 = 1.5;
/// The timing is read again after this long (ms): the vblanks are
/// extrapolated from it in between.
const PACE_TIMING_MS: f64 = 500.0;
/// No DWM timing: frames this far apart (ms).
const PACE_FALLBACK_MS: f64 = 1000.0 / 120.0;

impl Pacer {
    pub(crate) const fn new() -> Pacer {
        Pacer { made_for: None, cost: 2.0, timing: None }
    }

    /// From `now` (ms), with a vblank at `vblank` and the refresh
    /// `period` (ms): how long to wait before drawing (0: now) and the
    /// vblank (its time) the frame will be for.
    pub(crate) fn plan(&self, now: f64, vblank: f64, period: f64) -> (f64, f64) {
        let period = period.max(1.0);
        let lead = (self.cost + PACE_MARGIN_MS).clamp(0.0, period);
        // The vblank that ends the interval `now` is in; one frame each.
        let mut target = vblank + (((now - vblank) / period).floor() + 1.0) * period;
        if self.made_for.is_some_and(|m| m > target - period / 2.0) {
            target += period;
        }
        ((target - lead - now).max(0.0), target)
    }

    /// A frame for the vblank at `target` was drawn; it took `ms`.
    pub(crate) fn made(&mut self, target: f64, ms: f64) {
        self.made_for = Some(target);
        // Up at once, down slowly: a slow frame is planned for again.
        let ms = ms.clamp(0.0, 50.0);
        self.cost = if ms > self.cost { ms } else { self.cost * 0.9 + ms * 0.1 };
    }

    /// The display's last vblank and period (ms), read from DWM now and
    /// then; a fixed cadence without it.
    fn timing(&mut self, now: f64) -> (f64, f64) {
        if let Some((vb, period, at)) = self.timing
            && now - at < PACE_TIMING_MS
        {
            return (vb, period);
        }
        let t = unsafe {
            let mut ti: windows::Win32::Graphics::Dwm::DWM_TIMING_INFO = std::mem::zeroed();
            ti.cbSize = std::mem::size_of_val(&ti) as u32;
            windows::Win32::Graphics::Dwm::DwmGetCompositionTimingInfo(HWND::default(), &mut ti).ok().and_then(|_| {
                let (vb, period) = (ti.qpcVBlank, ti.qpcRefreshPeriod);
                let f = qpc_freq();
                (period > 0 && f > 0.0).then(|| (vb as f64 * 1000.0 / f, period as f64 * 1000.0 / f))
            })
        };
        let (vb, period) = t.unwrap_or((0.0, PACE_FALLBACK_MS));
        self.timing = Some((vb, period, now));
        (vb, period)
    }
}

fn qpc_freq() -> f64 {
    let mut f = 0i64;
    // SAFETY: writes an i64 we own.
    unsafe {
        let _ = windows::Win32::System::Performance::QueryPerformanceFrequency(&mut f);
    }
    f as f64
}

/// The noted move's frame: how long until it is due (ms; 0 = now), or
/// `None` with no move noted.
fn move_wait() -> Option<(f64, f64)> {
    if !move_noted() {
        return None;
    }
    let now = crate::perf::qpc_ms();
    Some(PACER.with_borrow_mut(|p| {
        let (vb, period) = p.timing(now);
        p.plan(now, vb, period)
    }))
}

/// Draw the noted move now: the newest moves still queued are noted
/// first (the freshest position), then the overlay handles it and paints.
unsafe fn draw_move(hwnd: HWND, target: f64) {
    unsafe {
        let mut m = MSG::default();
        while PeekMessageW(&mut m, Some(hwnd), WM_MOUSEMOVE, WM_MOUSEMOVE, PM_REMOVE).as_bool() {
            DispatchMessageW(&m);
        }
        let t0 = std::time::Instant::now();
        crate::perf::frame_for(target);
        let _ = SendMessageW(hwnd, WM_APP_MOVE, None, None);
        let ms = t0.elapsed().as_secs_f64() * 1000.0;
        PACER.with_borrow_mut(|p| p.made(target, ms));
    }
}

/// Re-arm the window timer (same id replaces the old interval).
pub fn retime(hwnd: HWND, ms: u64) {
    if hwnd.is_invalid() {
        return;
    }
    #[cfg(test)]
    tests::RETIMES.lock().unwrap_or_else(|e| e.into_inner()).push((hwnd.0 as isize, ms));
    unsafe {
        let _ = SetTimer(Some(hwnd), 1, ms as u32, None);
    }
}

unsafe extern "system" fn wndproc(
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
) -> LRESULT {
    unsafe {
        if msg == WM_NCCREATE {
            let cs = &*(lp.0 as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, cs.lpCreateParams as isize);
            return DefWindowProcW(hwnd, msg, wp, lp);
        }
        let slot = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut *mut dyn Driver;
        if slot.is_null() {
            return DefWindowProcW(hwnd, msg, wp, lp);
        }
        let drv = &mut **slot;
        #[cfg(test)]
        if msg == WM_ENTERSIZEMOVE {
            tests::MOVE_SIZE_LOOPS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        if let Some(r) = overlay_frame(hwnd, msg, wp, lp) {
            return r;
        }
        // Moves are noted and drawn once per refresh (see `Pacer`).
        if msg == WM_MOUSEMOVE {
            crate::perf::move_arrived();
            note_move(hwnd, x_of(lp), y_of(lp), crate::perf::move_time(wp.0));
            return LRESULT(0);
        }
        if msg == WM_APP_MOVE {
            if take_move(hwnd, drv) {
                let _ = UpdateWindow(hwnd);
                refresh_cursor(hwnd, drv);
            }
            return LRESULT(0);
        }
        if takes_move_first(msg) {
            take_move(hwnd, drv);
        }
        if msg == WM_DESTROY {
            NOTED.set(None);
            unpin(hwnd);
            drv.on_quit();
            forget_tick(hwnd);
            PostQuitMessage(0);
            return LRESULT(0);
        }
        let r = common(hwnd, msg, wp, lp, drv).unwrap_or_else(|| DefWindowProcW(hwnd, msg, wp, lp));
        // A key can change the cursor without a move (Ctrl over an object
        // draws over it: the cross, not the move cursor).
        if matches!(msg, WM_KEYDOWN | WM_KEYUP) {
            refresh_cursor(hwnd, drv);
        }
        r
    }
}

/// A button went up: the mouse capture goes only once no button is held
/// (a right click during a left drag keeps the drag's capture).
unsafe fn release_capture(wp: WPARAM) {
    // MK_LBUTTON | MK_RBUTTON | MK_MBUTTON: the buttons still down.
    if wp.0 & (0x0001 | 0x0002 | 0x0010) == 0 {
        unsafe {
            let _ = ReleaseCapture();
        }
    }
}

/// Input, timer and paint messages shared by the overlay and `run_window`;
/// `None` = not handled here.
unsafe fn common(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM, drv: &mut dyn Driver) -> Option<LRESULT> {
    unsafe {
        Some(match msg {
            WM_MOUSEMOVE => {
                // No automatic repaint: the driver calls `invalidate` itself
                // when a move actually changes what should be on screen.
                crate::perf::move_arrived();
                let _m = crate::perf::mouse_move(crate::perf::move_time(wp.0).map(|t| (t, t)));
                drv.on_event(Ev::Move {
                    x: x_of(lp),
                    y: y_of(lp),
                });
                LRESULT(0)
            }
            WM_LBUTTONDOWN => {
                let _ = SetCapture(hwnd);
                drv.on_event(Ev::Down {
                    x: x_of(lp),
                    y: y_of(lp),
                });
                request(hwnd, drv);
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                release_capture(wp);
                drv.on_event(Ev::Up {
                    x: x_of(lp),
                    y: y_of(lp),
                });
                request(hwnd, drv);
                LRESULT(0)
            }
            WM_MBUTTONDOWN | WM_MBUTTONUP => {
                let down = msg == WM_MBUTTONDOWN;
                if down {
                    let _ = SetCapture(hwnd);
                } else {
                    release_capture(wp);
                }
                drv.on_event(Ev::Middle { down, x: x_of(lp), y: y_of(lp) });
                request(hwnd, drv);
                LRESULT(0)
            }
            WM_RBUTTONDOWN | WM_RBUTTONUP => {
                let down = msg == WM_RBUTTONDOWN;
                if down {
                    let _ = SetCapture(hwnd);
                } else {
                    release_capture(wp);
                }
                drv.on_event(Ev::Right { down, x: x_of(lp), y: y_of(lp) });
                request(hwnd, drv);
                LRESULT(0)
            }
            WM_MOUSEWHEEL => {
                let delta = ((wp.0 >> 16) & 0xffff) as u16 as i16 as i32;
                let mut pt = POINT {
                    x: x_of(lp),
                    y: y_of(lp),
                };
                let _ = ScreenToClient(hwnd, &mut pt);
                drv.on_event(Ev::Wheel {
                    delta,
                    x: pt.x,
                    y: pt.y,
                });
                request(hwnd, drv);
                LRESULT(0)
            }
            WM_KEYDOWN | WM_KEYUP => {
                let repeat = msg == WM_KEYDOWN && (lp.0 & (1 << 30)) != 0;
                drv.on_event(Ev::Key {
                    vk: wp.0 as u32,
                    up: msg == WM_KEYUP,
                    repeat,
                    mods: Mods::current(),
                });
                request(hwnd, drv);
                LRESULT(0)
            }
            WM_CHAR => {
                drv.on_event(Ev::Char(wp.0 as u16));
                request(hwnd, drv);
                LRESULT(0)
            }
            WM_TIMER => {
                // Idle ticks (nothing animating) cost no frame.
                if drv.on_event(Ev::Timer) {
                    request(hwnd, drv);
                }
                LRESULT(0)
            }
            WM_PAINT => {
                let t0 = std::time::Instant::now();
                let rects = update_rects(hwnd);
                let mut ps: PAINTSTRUCT = std::mem::zeroed();
                let hdc = BeginPaint(hwnd, &mut ps);
                let rc = ps.rcPaint;
                let rects = if rects.is_empty() { vec![[rc.left, rc.top, rc.right, rc.bottom]] } else { rects };
                crate::perf::update(rects.len(), rects.iter().map(|r| (r[2] - r[0]) as u64 * (r[3] - r[1]) as u64).sum());
                if !drv.paint(hdc, &rects)
                    && let Some(fb) = drv.frame()
                {
                    present(hdc, fb);
                }
                let _ = EndPaint(hwnd, &ps);
                crate::perf::painted(t0);
                LRESULT(0)
            }
            WM_SETCURSOR => {
                if let Ok(c) = LoadCursorW(None, cursor_id(drv.cursor())) {
                    let _ = SetCursor(Some(c));
                }
                LRESULT(1)
            }
            WM_ERASEBKGND => LRESULT(1),
            _ => return None,
        })
    }
}

/// RGBA <-> BGRA: swap the R and B bytes of every pixel in place (its own
/// inverse), one pass, no allocation.
/// (`BI_BITFIELDS` masks would let GDI read RGBA directly, but its
/// conversion path is ~40x slower than this swap plus a BI_RGB blit.)
pub(crate) fn swap_rb(buf: &mut [u8]) {
    // Two pixels per step (the release profile is size-optimised, so no
    // auto-vectorisation): keep G/A, exchange the R and B bytes.
    let (b8, bt) = buf.as_chunks_mut::<8>();
    for o in b8.iter_mut() {
        let v = u64::from_le_bytes(*o);
        let ga = v & 0xFF00_FF00_FF00_FF00;
        let r = v & 0x0000_00FF_0000_00FF;
        let b = v & 0x00FF_0000_00FF_0000;
        *o = (ga | (r << 16) | (b >> 16)).to_le_bytes();
    }
    for o in bt.as_chunks_mut::<4>().0.iter_mut() {
        *o = [o[2], o[1], o[0], o[3]];
    }
}

/// Present an unpremultiplied RGBA framebuffer as top-down BGRA.
///
/// Converts `fb` to BGRA in place (no staging copy), so its contents are
/// BGRA afterwards. Sound because every paint (WM_PAINT) calls
/// `Driver::frame()` first, which rewrites the whole buffer; nothing
/// presents the same frame twice.
fn present(hdc: windows::Win32::Graphics::Gdi::HDC, fb: &mut PixBuf) {
    let (w, h) = fb.dimensions();
    if w == 0 || h == 0 {
        return;
    }
    let mut bmi: BITMAPINFO = unsafe { std::mem::zeroed() };
    bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
    bmi.bmiHeader.biWidth = w as i32;
    bmi.bmiHeader.biHeight = -(h as i32); // top-down
    bmi.bmiHeader.biPlanes = 1;
    bmi.bmiHeader.biBitCount = 32;
    bmi.bmiHeader.biCompression = BI_RGB.0;
    swap_rb(fb.as_raw_mut());
    unsafe {
        StretchDIBits(
            hdc,
            0,
            0,
            w as i32,
            h as i32,
            0,
            0,
            w as i32,
            h as i32,
            Some(fb.as_raw().as_ptr() as *const _),
            &bmi,
            DIB_RGB_COLORS,
            SRCCOPY,
        );
    }
}

/// Where each overlay belongs (`show_at`): `(hwnd, [x, y, w, h])`.
static PINS: std::sync::Mutex<Vec<(usize, [i32; 4])>> = std::sync::Mutex::new(Vec::new());

fn pins() -> std::sync::MutexGuard<'static, Vec<(usize, [i32; 4])>> {
    PINS.lock().unwrap_or_else(|e| e.into_inner())
}

fn pin(hwnd: HWND, at: [i32; 4]) {
    let mut p = pins();
    p.retain(|e| e.0 != hwnd.0 as usize);
    p.push((hwnd.0 as usize, at));
}

fn unpin(hwnd: HWND) {
    pins().retain(|e| e.0 != hwnd.0 as usize);
}

/// The overlay's frame is ours alone. Window-zoning tools (FancyZones,
/// VirtualZones: Shift+drag snaps the dragged window into a zone) and
/// Aero Snap move or resize foreground windows from outside; the overlay
/// must cover exactly the shot (a moved overlay drew the shot shifted and
/// half cut off). So: every position/size change is pinned back to the
/// `show_at` rect, hit tests are always client (no caption or border to
/// start a system move/size loop), the window menu's move/size/maximize/
/// minimize/restore do nothing, and a DPI change keeps the rect.
/// `None` = not an overlay frame message.
unsafe fn overlay_frame(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
    match msg {
        WM_WINDOWPOSCHANGING => {
            let at = pins().iter().find(|e| e.0 == hwnd.0 as usize).map(|e| e.1)?;
            // SAFETY: lParam of WM_WINDOWPOSCHANGING is a valid WINDOWPOS.
            let pos = unsafe { &mut *(lp.0 as *mut WINDOWPOS) };
            if !pos.flags.contains(SWP_NOMOVE) {
                (pos.x, pos.y) = (at[0], at[1]);
            }
            if !pos.flags.contains(SWP_NOSIZE) {
                (pos.cx, pos.cy) = (at[2], at[3]);
            }
            Some(LRESULT(0))
        }
        WM_NCHITTEST => Some(LRESULT(HTCLIENT as isize)),
        WM_SYSCOMMAND => {
            let cmd = (wp.0 & 0xFFF0) as u32;
            [SC_MOVE, SC_SIZE, SC_MAXIMIZE, SC_MINIMIZE, SC_RESTORE].contains(&cmd).then_some(LRESULT(0))
        }
        WM_DPICHANGED => Some(LRESULT(0)),
        _ => None,
    }
}

/// Position + show the overlay at an exact physical rect and take focus.
pub fn show_at(hwnd: HWND, x: i32, y: i32, w: i32, h: i32) {
    if hwnd.is_invalid() {
        return;
    }
    pin(hwnd, [x, y, w, h]);
    unsafe {
        let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), x, y, w, h, SWP_SHOWWINDOW);
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
    }
}

pub fn hide(hwnd: HWND) {
    unsafe {
        let _ = ShowWindow(hwnd, SW_HIDE);
    }
}

pub fn close(hwnd: HWND) {
    unsafe {
        let _ = DestroyWindow(hwnd);
    }
}

/// Create the (initially hidden) overlay and pump messages until quit.
/// Returns when the window is destroyed.
pub fn run(driver: &mut dyn Driver) -> i32 {
    unsafe {
        register_class();
        let h = module_handle().unwrap_or(core::ptr::null_mut());
        let slot: Box<*mut dyn Driver> = Box::new(driver);
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            w!("rustshot_overlay"),
            w!("Rustshot"),
            WS_POPUP,
            -32000,
            -32000,
            400,
            300,
            None,
            None,
            Some(HINSTANCE(h)),
            Some((&raw const *slot) as *const core::ffi::c_void),
        );
        match hwnd {
            Ok(hwnd) => {
                driver.on_create(hwnd);
                let _ = SetTimer(Some(hwnd), 1, tick_ms(hwnd) as u32, None);
                overlay_loop(hwnd);
                let _ = DestroyWindow(hwnd);
                0
            }
            Err(e) => {
                eprintln!("failed to create overlay window: {e}");
                1
            }
        }
    }
}

/// The overlay's message loop until WM_QUIT: everything queued is
/// handled as it comes; a noted move is drawn when its frame is due, also
/// in the middle of a stream of input (a fast mouse never starves it).
unsafe fn overlay_loop(hwnd: HWND) {
    use windows::Win32::System::Threading::{CreateWaitableTimerExW, SetWaitableTimer, CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, INFINITE, TIMER_ALL_ACCESS};
    unsafe {
        // A wait's timeout follows the system tick (15.6 ms by default):
        // a high-resolution timer wakes the loop on time for the frame.
        let timer = CreateWaitableTimerExW(None, PCWSTR::null(), CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, TIMER_ALL_ACCESS.0)
            .or_else(|_| CreateWaitableTimerExW(None, PCWSTR::null(), 0, TIMER_ALL_ACCESS.0))
            .ok();
        let mut msg = MSG::default();
        'outer: loop {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if msg.message == WM_QUIT {
                    break 'outer;
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
                if let Some((wait, target)) = move_wait()
                    && wait <= 0.0
                {
                    draw_move(hwnd, target);
                }
            }
            match move_wait() {
                Some((wait, target)) if wait <= 0.0 => draw_move(hwnd, target),
                Some((wait, _)) => {
                    // Until the next message, or the move's frame.
                    let due = -((wait * 10_000.0).ceil() as i64).max(1);
                    match timer.filter(|t| SetWaitableTimer(*t, &due, 0, None, None, false).is_ok()) {
                        Some(t) => MsgWaitForMultipleObjectsEx(Some(&[t]), INFINITE, QS_ALLINPUT, MWMO_INPUTAVAILABLE),
                        None => MsgWaitForMultipleObjectsEx(None, (wait.ceil() as u32).max(1), QS_ALLINPUT, MWMO_INPUTAVAILABLE),
                    };
                }
                None => {
                    MsgWaitForMultipleObjectsEx(None, INFINITE, QS_ALLINPUT, MWMO_INPUTAVAILABLE);
                }
            }
        }
        if let Some(t) = timer {
            let _ = windows::Win32::Foundation::CloseHandle(t);
        }
    }
}

// --- decorated window (run_window) ----------------------------------------

/// State behind GWLP_USERDATA of a `run_window` window (lives on
/// `run_window`'s stack frame for the whole loop).
struct WinState<'a> {
    drv: *mut (dyn Driver + 'a),
    /// Set right before `on_create`: CreateWindowEx already sends WM_SIZE
    /// etc., which must not reach a driver that has no handle yet.
    ready: core::cell::Cell<bool>,
    /// WM_DESTROY seen: the loop ends.
    done: core::cell::Cell<bool>,
    /// Last size sent as `Ev::Resize` (WM_SIZE repeats it on show/restore).
    size: core::cell::Cell<(u32, u32)>,
    min: (u32, u32),
    resizable: bool,
    style: WINDOW_STYLE,
    ex_style: WINDOW_EX_STYLE,
    /// The title bar colours last applied (`Driver::frame_colors`).
    frame: core::cell::Cell<Option<FrameColors>>,
    /// The UI language generation the window last drew in.
    lang: core::cell::Cell<u64>,
}

/// The native frame for `controls`: always a caption with a system menu
/// (close, Alt+F4, Alt+Space); a minimize box and a maximize box where
/// asked; a sizing border only for resizable windows. On the taskbar and
/// in Alt+Tab (`WS_EX_APPWINDOW`) with the class icon.
pub(crate) fn frame_style(controls: Controls, resizable: bool) -> (WINDOW_STYLE, WINDOW_EX_STYLE) {
    let mut style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU;
    if controls.minimize() {
        style |= WS_MINIMIZEBOX;
    }
    if controls.maximize() && resizable {
        style |= WS_MAXIMIZEBOX;
    }
    if resizable {
        style |= WS_THICKFRAME;
    }
    (style, WS_EX_APPWINDOW)
}

/// `0x00BBGGRR` for DWM.
pub(crate) fn colorref([r, g, b]: [u8; 3]) -> u32 {
    u32::from_le_bytes([r, g, b, 0])
}

/// Theme the OS title bar: dark/light caption buttons and text (Windows 10
/// 2004+ and 11), the caption and border in the design's colours (Windows
/// 11 22000+; older builds refuse the attribute and keep theirs). `None`:
/// the OS defaults again.
unsafe fn apply_frame(hwnd: HWND, c: Option<&FrameColors>) {
    use windows::Win32::Graphics::Dwm::{
        DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_CAPTION_COLOR, DWMWA_USE_IMMERSIVE_DARK_MODE,
    };
    /// DWMWA_COLOR_DEFAULT.
    const DEFAULT: u32 = 0xFFFF_FFFF;
    unsafe {
        let set = |a, v: u32| DwmSetWindowAttribute(hwnd, a, (&raw const v).cast(), 4);
        let _ = set(DWMWA_USE_IMMERSIVE_DARK_MODE, u32::from(c.is_some_and(|c| c.dark)));
        let _ = set(DWMWA_CAPTION_COLOR, c.map_or(DEFAULT, |c| colorref(c.caption)));
        let _ = set(DWMWA_BORDER_COLOR, c.map_or(DEFAULT, |c| colorref(c.border)));
    }
}

/// Apply the driver's title bar colours when they changed since the last
/// time (a theme switch in Settings repaints, so this runs on paint).
unsafe fn sync_frame(hwnd: HWND, st: &WinState<'_>, drv: &dyn Driver) {
    let want = drv.frame_colors();
    if st.frame.get() != want {
        st.frame.set(want);
        unsafe { apply_frame(hwnd, want.as_ref()) };
    }
}

const WINDOW_CLASS: PCWSTR = w!("rustshot_window");
/// Registered successfully (a failure is not cached: the next call retries).
static WINDOW_CLASS_OK: std::sync::Mutex<bool> = std::sync::Mutex::new(false);

/// Register the decorated-window class once (any thread may be first).
/// Icons come from the exe's icon resource (ID 1, embedded by build.rs);
/// shared handles, so nothing is destroyed. Builds without the resource
/// (tests, non-MSVC) fall back to the default icon.
unsafe fn register_window_class(h: HINSTANCE) -> anyhow::Result<()> {
    let mut ok = WINDOW_CLASS_OK.lock().unwrap_or_else(|e| e.into_inner());
    if *ok {
        return Ok(());
    }
    unsafe {
        let icon = |cx, cy| {
            LoadImageW(Some(h), PCWSTR(std::ptr::without_provenance(1)) /* MAKEINTRESOURCE(1) */, IMAGE_ICON, cx, cy, LR_SHARED)
                .map(|i| HICON(i.0))
                .unwrap_or_default()
        };
        let mut wc: WNDCLASSEXW = std::mem::zeroed();
        wc.cbSize = std::mem::size_of::<WNDCLASSEXW>() as u32;
        wc.style = CS_HREDRAW | CS_VREDRAW;
        wc.lpfnWndProc = Some(wndproc_window);
        wc.hInstance = h;
        wc.hCursor = LoadCursorW(None, IDC_ARROW).unwrap_or_default();
        wc.hIcon = icon(GetSystemMetrics(SM_CXICON), GetSystemMetrics(SM_CYICON));
        wc.hIconSm = icon(GetSystemMetrics(SM_CXSMICON), GetSystemMetrics(SM_CYSMICON));
        wc.lpszClassName = WINDOW_CLASS;
        if !class_ok(RegisterClassExW(&wc)) {
            anyhow::bail!("RegisterClassExW failed: {}", windows::core::Error::from_thread());
        }
    }
    *ok = true;
    Ok(())
}

/// UI scale of a `run_window` window: its DPI (1.0 = 96 dpi) times the
/// UI size; the logical -> physical px factor its driver draws at.
pub fn scale(hwnd: HWND) -> f32 {
    crate::uisize::apply(match unsafe { GetDpiForWindow(hwnd) } {
        0 => 1.0,
        dpi => dpi as f32 / 96.0,
    })
}

/// Posted by [`resize`]: the new client size (logical px) in the params.
const WM_RESIZE_CLIENT: u32 = WM_APP + 0x31;

/// Give a `run_window` window the client size `w`×`h` (logical px) at its
/// current [`scale`] (the UI size changed), keeping its centre on its
/// monitor's work area. Posted, so it is safe from a driver callback:
/// the `Ev::Resize` follows from the message loop.
pub fn resize(hwnd: HWND, w: u32, h: u32) {
    unsafe {
        let _ = PostMessageW(Some(hwnd), WM_RESIZE_CLIENT, WPARAM(w as usize), LPARAM(h as isize));
    }
}

/// [`resize`]'s message: the outer rect for the client size, centred where
/// the window was.
unsafe fn resize_client(hwnd: HWND, st: &WinState<'_>, w: u32, h: u32) {
    unsafe {
        let dpi = GetDpiForWindow(hwnd).max(96);
        let s = scale(hwnd);
        let mut rc = RECT { left: 0, top: 0, right: to_phys(w, s) as i32, bottom: to_phys(h, s) as i32 };
        let _ = AdjustWindowRectExForDpi(&mut rc, st.style, false, st.ex_style, dpi);
        let (ow, oh) = (rc.right - rc.left, rc.bottom - rc.top);
        let mut wr = RECT::default();
        let _ = GetWindowRect(hwnd, &mut wr);
        let (cx, cy) = ((wr.left + wr.right) / 2, (wr.top + wr.bottom) / 2);
        let mon = MonitorFromPoint(POINT { x: cx, y: cy }, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(mon, &mut mi);
        let wa = mi.rcWork;
        let x = (cx - ow / 2).min(wa.right - ow).max(wa.left);
        let y = (cy - oh / 2).min(wa.bottom - oh).max(wa.top);
        let _ = SetWindowPos(hwnd, None, x, y, ow, oh, SWP_NOZORDER | SWP_NOACTIVATE);
    }
}

unsafe extern "system" fn wndproc_window(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        if msg == WM_NCCREATE {
            let cs = &*(lp.0 as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, cs.lpCreateParams as isize);
            return DefWindowProcW(hwnd, msg, wp, lp);
        }
        let st = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const WinState<'static>;
        if st.is_null() {
            return DefWindowProcW(hwnd, msg, wp, lp);
        }
        let st = &*st;
        match msg {
            WM_GETMINMAXINFO => {
                if st.resizable {
                    let dpi = GetDpiForWindow(hwnd).max(96);
                    let s = crate::uisize::apply(dpi as f32 / 96.0);
                    let mut rc = RECT {
                        left: 0,
                        top: 0,
                        right: to_phys(st.min.0, s) as i32,
                        bottom: to_phys(st.min.1, s) as i32,
                    };
                    let _ = AdjustWindowRectExForDpi(&mut rc, st.style, false, st.ex_style, dpi);
                    let mmi = &mut *(lp.0 as *mut MINMAXINFO);
                    mmi.ptMinTrackSize = POINT { x: rc.right - rc.left, y: rc.bottom - rc.top };
                }
                return LRESULT(0);
            }
            WM_DPICHANGED => {
                // Moved onto a monitor with another scale: take the size
                // Windows suggests (keeps the logical size); WM_SIZE follows.
                let r = &*(lp.0 as *const RECT);
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    r.left,
                    r.top,
                    r.right - r.left,
                    r.bottom - r.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
                return LRESULT(0);
            }
            // DefWindowProc maximizes on any SC_MAXIMIZE, even without a
            // maximize box (a fixed-size window filling the screen): the
            // shell's own paths (Win+Up, a title double-click) honour the
            // style, posted/sent commands from other tools do not.
            WM_SYSCOMMAND if (wp.0 & 0xFFF0) as u32 == SC_MAXIMIZE && st.style.0 & WS_MAXIMIZEBOX.0 == 0 => {
                return LRESULT(0);
            }
            WM_RESIZE_CLIENT => {
                resize_client(hwnd, st, wp.0 as u32, lp.0 as u32);
                return LRESULT(0);
            }
            WM_NCDESTROY => {
                // The state dies with run_window's frame: never reach it again.
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                return DefWindowProcW(hwnd, msg, wp, lp);
            }
            _ => {}
        }
        if !st.ready.get() {
            return DefWindowProcW(hwnd, msg, wp, lp);
        }
        let drv = &mut *st.drv;
        match msg {
            WM_CLOSE => {
                // Never DefWindowProc (it would destroy): the driver decides.
                drv.on_event(Ev::Close);
                if !st.done.get() {
                    request(hwnd, drv);
                }
                LRESULT(0)
            }
            WM_SIZE => {
                if wp.0 != SIZE_MINIMIZED as usize {
                    let size = ((lp.0 & 0xffff) as u32, ((lp.0 >> 16) & 0xffff) as u32);
                    if st.size.replace(size) != size {
                        drv.on_event(Ev::Resize(size.0, size.1));
                    }
                    invalidate(hwnd);
                }
                LRESULT(0)
            }
            WM_SETFOCUS | WM_KILLFOCUS => {
                drv.on_event(Ev::Focus(msg == WM_SETFOCUS));
                request(hwnd, drv);
                LRESULT(0)
            }
            WM_TIMER if crate::i18n::generation() != st.lang.get() => {
                // The UI language switched: lay out again, retitle, repaint.
                let mut seen = st.lang.get();
                if let Some(title) = crate::wind::relocalize(&mut seen, drv, st.size.get()) {
                    if let Some(t) = title {
                        set_title(hwnd, &t);
                    }
                    invalidate(hwnd);
                }
                st.lang.set(seen);
                common(hwnd, msg, wp, lp, drv).unwrap_or_default()
            }
            WM_PAINT => {
                sync_frame(hwnd, st, drv);
                common(hwnd, msg, wp, lp, drv).unwrap_or_default()
            }
            // Borders/caption keep their resize/arrow cursors.
            WM_SETCURSOR if (lp.0 & 0xffff) as u32 != HTCLIENT => DefWindowProcW(hwnd, msg, wp, lp),
            WM_DESTROY => {
                st.done.set(true);
                drv.on_quit();
                forget_tick(hwnd);
                LRESULT(0)
            }
            _ => common(hwnd, msg, wp, lp, drv).unwrap_or_else(|| DefWindowProcW(hwnd, msg, wp, lp)),
        }
    }
}

/// Set a `run_window` window's title bar text.
pub fn set_title(hwnd: HWND, title: &str) {
    let w: Vec<u16> = title.encode_utf16().chain(Some(0)).collect();
    unsafe {
        let _ = windows::Win32::UI::WindowsAndMessaging::SetWindowTextW(hwnd, PCWSTR(w.as_ptr()));
    }
}

/// Bring a `run_window` window to the front, restoring it if minimised
/// (best effort: Windows may only flash its taskbar button).
pub fn raise(hwnd: HWND) {
    unsafe {
        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }
        let _ = SetForegroundWindow(hwnd);
    }
}

/// Open a normal decorated, per-monitor-DPI-aware top-level window centred
/// on the monitor under the cursor and pump this thread's messages until
/// it is destroyed. Callable from any thread (each thread has its own
/// queue): the close button only sends `Ev::Close`; the driver ends the
/// loop with `wind::close(hwnd)` from one of its callbacks. The driver
/// gets `Ev::Resize` (physical px) right after `on_create` and on every
/// resize; `wind::scale(hwnd)` gives the DPI factor. A `WM_QUIT` that
/// arrives meanwhile (another window on this thread quitting) closes the
/// window and is re-posted for the caller's own loop.
pub fn run_window(spec: WindowSpec, drv: &mut dyn Driver) -> anyhow::Result<()> {
    unsafe {
        // Per thread, so it holds even where the process never opted in.
        let prev = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let r = window_loop(&spec, drv);
        if !prev.0.is_null() {
            SetThreadDpiAwarenessContext(prev);
        }
        r
    }
}

unsafe fn window_loop(spec: &WindowSpec, drv: &mut dyn Driver) -> anyhow::Result<()> {
    unsafe {
        let h = HINSTANCE(module_handle()?);
        register_window_class(h)?;
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let mon = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(mon, &mut mi);
        let (mut dx, mut dy) = (96u32, 96u32);
        let dpi = if GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy).is_ok() { dx.max(96) } else { 96 };
        // The UI size on top of the monitor's DPI (as `scale` gives it).
        let s = crate::uisize::apply(dpi as f32 / 96.0);
        let (style, ex_style) = frame_style(spec.controls, spec.resizable);
        let mut rc = RECT { left: 0, top: 0, right: to_phys(spec.w, s) as i32, bottom: to_phys(spec.h, s) as i32 };
        let _ = AdjustWindowRectExForDpi(&mut rc, style, false, ex_style, dpi);
        let (ow, oh) = (rc.right - rc.left, rc.bottom - rc.top);
        let wa = mi.rcWork;
        let (x, y) = centre_in((wa.left, wa.top, wa.right - wa.left, wa.bottom - wa.top), ow, oh);
        let title: Vec<u16> = spec.title.encode_utf16().chain(Some(0)).collect();
        let st = WinState {
            drv: drv as *mut (dyn Driver + '_),
            ready: core::cell::Cell::new(false),
            done: core::cell::Cell::new(false),
            size: core::cell::Cell::new((0, 0)),
            min: spec.min,
            resizable: spec.resizable,
            style,
            ex_style,
            frame: core::cell::Cell::new(None),
            lang: core::cell::Cell::new(crate::i18n::generation()),
        };
        let hwnd = CreateWindowExW(
            ex_style,
            WINDOW_CLASS,
            PCWSTR(title.as_ptr()),
            style,
            x,
            y,
            ow,
            oh,
            None,
            None,
            Some(h),
            Some(&raw const st as *const core::ffi::c_void),
        )?;
        st.ready.set(true);
        {
            // Rounded corners on Windows 11 (the default for a captioned
            // window, asked for so small dialogs get them too).
            use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND};
            let round = DWMWCP_ROUND;
            let _ = DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, (&raw const round).cast(), 4);
        }
        (*st.drv).on_create(hwnd);
        if !st.done.get() {
            let mut cr = RECT::default();
            let _ = GetClientRect(hwnd, &mut cr);
            let size = (cr.right.max(0) as u32, cr.bottom.max(0) as u32);
            st.size.set(size);
            (*st.drv).on_event(Ev::Resize(size.0, size.1));
        }
        if !st.done.get() {
            // Themed before it is first shown: no white title bar flash.
            sync_frame(hwnd, &st, &*st.drv);
            let _ = SetTimer(Some(hwnd), 1, tick_ms(hwnd) as u32, None);
            // Tests never steal the user's focus.
            if cfg!(test) {
                let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            } else {
                let _ = ShowWindow(hwnd, SW_SHOWNORMAL);
                let _ = SetForegroundWindow(hwnd);
            }
            let _ = UpdateWindow(hwnd);
        }
        let mut quit = None;
        let mut failed = None;
        let mut msg = MSG::default();
        while !st.done.get() {
            match GetMessageW(&mut msg, None, 0, 0).0 {
                0 => {
                    quit = Some(msg.wParam.0 as i32);
                    break;
                }
                -1 => {
                    failed = Some(windows::core::Error::from_thread());
                    break;
                }
                _ => {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
        }
        if !st.done.get() {
            let _ = DestroyWindow(hwnd);
        }
        if let Some(code) = quit {
            PostQuitMessage(code);
        }
        if let Some(e) = failed {
            anyhow::bail!("run_window message loop failed: {e}");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A selection drag with the magnifier, through a real GDI region (the
    /// banding `InvalidateRgn` makes): the paint gets at most 32 rects that
    /// cover the damage, far smaller than the bounding box (the whole
    /// 3840x2160 overlay, which the crosshair spans).
    #[test]
    fn many_region_pieces_coalesce_not_to_the_bounding_box() {
        let (w, h) = (3840, 2160);
        // Two frames of damage: the crosshair bands, the selection strips
        // and handles, the label, the loupe and its caption.
        let mut dmg: Vec<[i32; 4]> = vec![[0, 600, w, 606], [900, 0, 908, h], [0, 640, w, 646], [960, 0, 968, h]];
        for (x, y) in [(400, 300), (900, 600), (960, 640)] {
            dmg.push([400, 300, x + 4, 304]);
            dmg.push([x - 2, 300, x + 2, y + 2]);
            dmg.push([380, 280, 420, 320]);
            dmg.push([x - 20, y - 20, x + 20, y + 20]);
            dmg.push([x + 30, y + 30, x + 290, y + 360]);
            dmg.push([400, 270, 560, 296]);
        }
        unsafe {
            let all = CreateRectRgn(0, 0, 0, 0);
            let one = CreateRectRgn(0, 0, 0, 0);
            for r in &dmg {
                let _ = SetRectRgn(one, r[0], r[1], r[2], r[3]);
                CombineRgn(Some(all), Some(all), Some(one), RGN_OR);
            }
            let pieces = region_rects(all);
            let _ = DeleteObject(one.into());
            let _ = DeleteObject(all.into());
            assert!(pieces.len() > MAX_PAINT_RECTS, "a banded region of {} pieces", pieces.len());
            let got = coalesce(pieces.clone(), MAX_PAINT_RECTS);
            assert!(got.len() <= MAX_PAINT_RECTS);
            let area = |r: &[i32; 4]| (r[2] - r[0]) as i64 * (r[3] - r[1]) as i64;
            let a: i64 = got.iter().map(area).sum();
            let d: i64 = pieces.iter().map(area).sum();
            assert!(a < (w as i64 * h as i64) / 8, "{a} px painted for {d} px of damage");
            // Every piece is covered.
            for p in &pieces {
                assert!(got.iter().any(|g| g[0] <= p[0] && g[1] <= p[1] && p[2] <= g[2] && p[3] <= g[3]), "{p:?} not covered");
            }
        }
        // Few pieces pass through as they are.
        let few = vec![[0, 0, 4, 4], [10, 10, 20, 20]];
        assert_eq!(coalesce(few.clone(), MAX_PAINT_RECTS), few);
    }
    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GdiFlush,
        SelectObject,
    };

    /// `present` into a BGRA DIB section: every channel lands in place.
    #[test]
    #[ignore = "GDI rendering check (no window)"]
    fn present_colors_round_trip() {
        let px: [[u8; 4]; 8] = [
            [255, 0, 0, 255],
            [0, 255, 0, 255],
            [0, 0, 255, 255],
            [255, 255, 255, 255],
            [12, 34, 56, 255],
            [200, 100, 50, 255],
            [1, 2, 3, 255],
            [250, 128, 7, 255],
        ];
        let mut fb = PixBuf::from_raw(4, 2, px.concat()).unwrap();
        unsafe {
            let dc = CreateCompatibleDC(None);
            assert!(!dc.is_invalid());
            let mut bmi: BITMAPINFO = std::mem::zeroed();
            bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            bmi.bmiHeader.biWidth = 4;
            bmi.bmiHeader.biHeight = -2;
            bmi.bmiHeader.biPlanes = 1;
            bmi.bmiHeader.biBitCount = 32;
            bmi.bmiHeader.biCompression = BI_RGB.0;
            let mut bits: *mut core::ffi::c_void = core::ptr::null_mut();
            let bmp = CreateDIBSection(Some(dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0).unwrap();
            let old = SelectObject(dc, bmp.into());
            present(dc, &mut fb);
            let _ = GdiFlush();
            let out = std::slice::from_raw_parts(bits as *const u8, 4 * 2 * 4).to_vec();
            SelectObject(dc, old);
            let _ = DeleteObject(bmp.into());
            let _ = DeleteDC(dc);
            for (i, p) in px.iter().enumerate() {
                let q = &out[i * 4..i * 4 + 3];
                assert_eq!([q[2], q[1], q[0]], [p[0], p[1], p[2]], "pixel {i}: BGRA {q:?}");
            }
        }
    }

    /// WM_ENTERSIZEMOVE seen by an overlay (a system move/size loop began).
    pub(super) static MOVE_SIZE_LOOPS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

    /// Every `(hwnd, ms)` the backend's `retime` was asked for.
    pub(super) static RETIMES: std::sync::Mutex<Vec<(isize, u64)>> = std::sync::Mutex::new(Vec::new());

    use crate::wind::test_window_lock as windows_lock;

    /// Resize factors applied to the initial outer rect (pairwise distinct,
    /// DPI independent: `min` is 1x1 in the test spec).
    const RESIZES: [f32; 3] = [1.1, 1.25, 0.9];

    /// Records what `run_window` delivers; ignores the first close request
    /// (the window must stay open) and closes on the second. On the first
    /// close a helper thread resizes and repaints the window (cross-thread,
    /// so the messages arrive through this loop, never re-entrantly inside
    /// a driver callback), then posts the second WM_CLOSE.
    struct Closer {
        hwnd: HWND,
        fb: PixBuf,
        created: bool,
        first: Option<&'static str>,
        first_size: Option<(u32, u32)>,
        dpi: u32,
        size: (u32, u32),
        resizes: u32,
        closes: u32,
        frames: u32,
        quit: bool,
        helper: Option<std::thread::JoinHandle<()>>,
    }

    impl Driver for Closer {
        fn on_create(&mut self, hwnd: HWND) {
            self.hwnd = hwnd;
            self.created = true;
            unsafe {
                self.dpi = GetDpiForWindow(hwnd);
                let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
        }
        fn on_event(&mut self, ev: Ev) -> bool {
            self.first.get_or_insert(match ev {
                Ev::Resize(..) => "resize",
                _ => "other",
            });
            match ev {
                Ev::Resize(w, h) => {
                    self.resizes += 1;
                    self.first_size.get_or_insert((w, h));
                    self.size = (w, h);
                    self.fb = PixBuf::new(w, h);
                }
                Ev::Close => {
                    self.closes += 1;
                    if self.closes == 1 {
                        let raw = self.hwnd.0 as usize;
                        self.helper = Some(std::thread::spawn(move || unsafe {
                            use windows::Win32::Graphics::Gdi::{RedrawWindow, RDW_INVALIDATE, RDW_UPDATENOW};
                            let hwnd = HWND(raw as *mut core::ffi::c_void);
                            let mut r = RECT::default();
                            let _ = GetWindowRect(hwnd, &mut r);
                            let (ow, oh) = ((r.right - r.left) as f32, (r.bottom - r.top) as f32);
                            for f in RESIZES {
                                let (w, h) = ((ow * f).round() as i32, (oh * f).round() as i32);
                                let _ = SetWindowPos(hwnd, None, 0, 0, w, h, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE);
                                let _ = RedrawWindow(Some(hwnd), None, None, RDW_INVALIDATE | RDW_UPDATENOW);
                            }
                            let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
                        }));
                    } else {
                        close(self.hwnd);
                    }
                }
                _ => {}
            }
            false
        }
        fn frame(&mut self) -> Option<&mut PixBuf> {
            self.frames += 1;
            self.fb.as_raw_mut().fill(0x80);
            Some(&mut self.fb)
        }
        fn cursor(&self) -> Cursor {
            Cursor::Arrow
        }
        fn on_quit(&mut self) {
            self.quit = true;
        }
    }

    fn run_closer() -> Closer {
        let mut d = Closer {
            hwnd: HWND::default(),
            fb: PixBuf::new(1, 1),
            created: false,
            first: None,
            first_size: None,
            dpi: 0,
            size: (0, 0),
            resizes: 0,
            closes: 0,
            frames: 0,
            quit: false,
            helper: None,
        };
        let spec = WindowSpec {
            title: "rustshot wind test".into(),
            w: 400,
            h: 300,
            resizable: true,
            min: (1, 1),
            controls: Controls::MinMaxClose,
        };
        run_window(spec, &mut d).expect("window loop");
        if let Some(h) = d.helper.take() {
            h.join().expect("resize helper");
        }
        d
    }

    /// Records the client sizes a `run_window` window gets while the UI
    /// size changes under it: 150 % at open, then back to Auto set "from
    /// elsewhere" (the overlay's Ctrl+=): the next timer tick sees the
    /// scale differ and resizes, as Settings and the update dialog do.
    struct Sizer {
        hwnd: HWND,
        fb: PixBuf,
        dpi: u32,
        sizes: Vec<(u32, u32)>,
        sized_for: f32,
    }

    impl Driver for Sizer {
        fn on_create(&mut self, hwnd: HWND) {
            self.hwnd = hwnd;
            self.dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
        }
        fn on_event(&mut self, ev: Ev) -> bool {
            if let Ev::Resize(w, h) = ev {
                self.sizes.push((w, h));
                self.fb = PixBuf::new(w.max(1), h.max(1));
                match self.sizes.len() {
                    1 => {
                        follow_scale(self.hwnd, &mut self.sized_for, 400, 300);
                        crate::uisize::set(crate::uisize::UiSize::Auto);
                    }
                    _ => close(self.hwnd),
                }
            }
            if matches!(ev, Ev::Timer) {
                follow_scale(self.hwnd, &mut self.sized_for, 400, 300);
            }
            false
        }
        fn frame(&mut self) -> Option<&mut PixBuf> {
            Some(&mut self.fb)
        }
        fn cursor(&self) -> Cursor {
            Cursor::Arrow
        }
    }

    /// A dialog opens at its logical size times the monitor's DPI times
    /// the UI size, and `resize` gives it the size for a new UI size.
    #[test]
    fn run_window_follows_the_ui_size() {
        let _guard = windows_lock();
        let d = std::thread::spawn(|| {
            // The UI size is per thread under test: this window's alone.
            crate::uisize::set(crate::uisize::UiSize::Pct(150));
            let mut d = Sizer { hwnd: HWND::default(), fb: PixBuf::new(1, 1), dpi: 96, sizes: Vec::new(), sized_for: 0.0 };
            let spec = WindowSpec { title: "rustshot ui size test".into(), w: 400, h: 300, resizable: false, min: (400, 300), controls: Controls::MinClose };
            run_window(spec, &mut d).expect("window loop");
            (d.dpi, d.sizes)
        })
        .join()
        .expect("window thread");
        let (dpi, sizes) = d;
        let os = dpi as f32 / 96.0;
        let want = |k: f32| (to_phys(400, k), to_phys(300, k));
        assert_eq!(sizes.len(), 2, "{sizes:?}");
        assert_eq!(sizes[0], want(os * 1.5), "opened at 150 % of {os}");
        assert_eq!(sizes[1], want(os), "back to Auto");
    }

    /// `run_window` on a non-main thread: the loop survives an ignored
    /// WM_CLOSE, ends when the driver closes, paints at least once, and
    /// repeated windows leave the GDI/USER object counts where they were.
    /// The counts are per process and every other test in this one may
    /// create GDI objects at any time, so they are taken in a child
    /// process running only `window_leak_probe`.
    #[test]
    fn run_window_closes_on_request_without_leaks() {
        let _guard = windows_lock();
        std::thread::spawn(|| {
            let d = run_closer();
            assert!(d.created);
            assert_eq!(d.first, Some("resize"), "first event is the initial size");
            let first = d.first_size.expect("initial Resize");
            let s = d.dpi.max(96) as f32 / 96.0;
            assert!(
                first.0 as f32 >= (400.0 * s).floor() && first.1 as f32 >= (300.0 * s).floor(),
                "initial client {first:?} at scale {s}"
            );
            assert_eq!(d.closes, 2, "the first close was ignored");
            assert_eq!(d.resizes, 1 + RESIZES.len() as u32, "initial size plus each SetWindowPos");
            assert!(d.quit, "on_quit ran");
            assert!(d.frames > RESIZES.len() as u32, "painted after every resize: {} frames", d.frames);
            assert!(unsafe { !IsWindow(Some(d.hwnd)).as_bool() }, "window destroyed");
        })
        .join()
        .expect("window thread");
        let here = module_path!().split_once("::").map_or(module_path!(), |m| m.1);
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args([&format!("{here}::window_leak_probe"), "--exact", "--ignored", "--nocapture", "--test-threads=1"])
            .env("RUSTSHOT_LEAK_PROBE", "1")
            .output()
            .expect("run the leak probe");
        let text = String::from_utf8_lossy(&out.stdout);
        println!("{text}");
        assert!(out.status.success(), "leak probe failed:\n{text}\n{}", String::from_utf8_lossy(&out.stderr));
        assert!(text.contains(LEAK_PROBE_DONE), "the probe ran to the end:\n{text}");
    }

    /// Printed by `window_leak_probe` once its checks passed.
    const LEAK_PROBE_DONE: &str = "rustshot-leak-probe: done";

    /// Run alone in a child process by the test above: windows in a row do
    /// not grow the GDI/USER object counts per window. Windows allocates
    /// some process-wide objects lazily and frees others a little late
    /// (IME/text-services windows, theme data), so an exact "no growth
    /// over N windows" check flakes under load; a leak of even one object
    /// per window shows as at least `PER_ROUND` over a round, while that
    /// background noise stays a few objects, so the bound sits between.
    #[test]
    #[ignore = "helper: run as a child process by run_window_closes_on_request_without_leaks"]
    fn window_leak_probe() {
        use windows::Win32::System::Threading::{
            GetGuiResources, OpenProcess, GR_GDIOBJECTS, GR_USEROBJECTS, PROCESS_QUERY_INFORMATION,
        };
        if std::env::var_os("RUSTSHOT_LEAK_PROBE").is_none() {
            return;
        }
        // A real handle: the GetCurrentProcess pseudo handle reads 0.
        let me = unsafe { OpenProcess(PROCESS_QUERY_INFORMATION, false, std::process::id()) }.expect("process handle");
        let count = || unsafe { (GetGuiResources(me, GR_GDIOBJECTS), GetGuiResources(me, GR_USEROBJECTS)) };
        const PER_ROUND: u32 = 20;
        const NOISE: u32 = PER_ROUND / 2;
        let window = || {
            let d = run_closer();
            assert_eq!(d.resizes, 1 + RESIZES.len() as u32);
        };
        // Warm-up: the first windows allocate process-wide caches.
        (0..3).for_each(|_| window());
        let mut seen = Vec::new();
        let ok = (0..3).any(|_| {
            let before = count();
            (0..PER_ROUND).for_each(|_| window());
            let after = count();
            seen.push((before, after));
            after.0 < before.0 + NOISE && after.1 < before.1 + NOISE
        });
        let _ = unsafe { windows::Win32::Foundation::CloseHandle(me) };
        println!("(gdi, user) objects before -> after {PER_ROUND} windows: {seen:?}");
        assert!(ok, "GDI/USER objects grew by {NOISE}+ over {PER_ROUND} windows ((gdi, user) before -> after): {seen:?}");
        println!("{LEAK_PROBE_DONE}");
    }

    /// Two `run_window` threads at once: the "overlay-style" window goes
    /// fast, the dialog goes fast then idle again; each keeps its own
    /// cadence (the dialog going idle must not slow the other window).
    #[test]
    fn run_window_cadence_is_per_window() {
        use crate::chan as mpsc;
        use std::time::{Duration, Instant};
        let _guard = windows_lock();
        RETIMES.lock().unwrap_or_else(|e| e.into_inner()).clear();
        struct Ticker {
            hwnd: HWND,
            fb: PixBuf,
            idle_again: bool,
            ready: mpsc::Sender<()>,
            go: mpsc::Receiver<()>,
            cadence: u64,
            start: Option<Instant>,
            ticks: u32,
        }
        impl Driver for Ticker {
            fn on_create(&mut self, hwnd: HWND) {
                self.hwnd = hwnd;
                set_fast_timer(hwnd, true);
                if self.idle_again {
                    set_fast_timer(hwnd, false);
                }
                // Both windows have toggled once the main thread says go;
                // a window that never reports makes the test fail, not hang.
                let _ = self.ready.send(());
                let _ = self.go.recv_timeout(Duration::from_secs(5));
                self.cadence = tick_ms(hwnd);
                self.start = Some(Instant::now());
            }
            fn on_event(&mut self, ev: Ev) -> bool {
                if let Ev::Timer = ev {
                    self.ticks += 1;
                    if self.start.is_some_and(|s| s.elapsed() >= Duration::from_millis(450)) {
                        close(self.hwnd);
                    }
                }
                false
            }
            fn frame(&mut self) -> Option<&mut PixBuf> {
                Some(&mut self.fb)
            }
            fn cursor(&self) -> Cursor {
                Cursor::Arrow
            }
        }
        let (ready_tx, ready_rx) = mpsc::channel();
        let mut gos = Vec::new();
        let (hw_tx, hw_rx) = mpsc::channel();
        let mut spawn = |idle_again: bool| {
            let (go_tx, go_rx) = mpsc::channel();
            gos.push(go_tx);
            let (ready, hw_tx) = (ready_tx.clone(), hw_tx.clone());
            std::thread::spawn(move || {
                let mut d = Ticker {
                    hwnd: HWND::default(),
                    fb: PixBuf::new(1, 1),
                    idle_again,
                    ready,
                    go: go_rx,
                    cadence: 0,
                    start: None,
                    ticks: 0,
                };
                let title = if idle_again { "rustshot dialog" } else { "rustshot animating" };
                let spec = WindowSpec { title: title.into(), w: 200, h: 150, resizable: false, min: (0, 0), controls: Controls::MinClose };
                run_window(spec, &mut d).expect("window loop");
                let _ = hw_tx.send((idle_again, d.hwnd.0 as isize));
                (d.cadence, d.ticks, tick_ms(d.hwnd))
            })
        };
        let (fast, dialog) = (spawn(false), spawn(true));
        drop(hw_tx);
        for i in 0..2 {
            ready_rx.recv_timeout(Duration::from_secs(5)).unwrap_or_else(|_| panic!("window {i} never became ready"));
        }
        for g in &gos {
            let _ = g.send(());
        }
        let (fast, dialog) = (fast.join().expect("fast window"), dialog.join().expect("dialog"));
        let hwnds: std::collections::HashMap<bool, isize> = hw_rx.try_iter().collect();
        let log = RETIMES.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let of = |h: isize| log.iter().filter(|r| r.0 == h).map(|r| r.1).collect::<Vec<_>>();
        assert_eq!(of(hwnds[&true]), [FAST_TICK_MS, SLOW_TICK_MS], "dialog retimed fast then slow");
        assert_eq!(of(hwnds[&false]), [FAST_TICK_MS], "the animating window was retimed once, never slowed");
        assert!(log.iter().all(|r| r.0 == hwnds[&true] || r.0 == hwnds[&false]), "no other window retimed: {log:?}");
        println!("(cadence, ticks in 450 ms, cadence after destroy): animating {fast:?}, dialog {dialog:?}");
        assert_eq!(fast.0, FAST_TICK_MS, "the dialog going idle left the other window fast");
        assert_eq!(dialog.0, SLOW_TICK_MS);
        assert!(fast.1 >= dialog.1 + 2, "fast window ticked {} times, dialog {}", fast.1, dialog.1);
        assert_eq!((fast.2, dialog.2), (SLOW_TICK_MS, SLOW_TICK_MS), "destroyed windows are forgotten");
    }

    /// Interactive: a 400x300 window rendering a filled rect; close it
    /// with its close button or Esc. `RUSTSHOT_TEST_AUTOCLOSE_MS=1500`
    /// samples the rect's centre pixel from the window DC, then posts
    /// WM_CLOSE (the close-button path) instead of waiting for a human.
    /// `cargo test interactive_window -- --ignored --nocapture`
    #[test]
    #[ignore = "interactive: opens a window"]
    fn interactive_window_renders_rect() {
        use windows::Win32::Graphics::Gdi::{GetDC, GetPixel, ReleaseDC};
        const FILL: [u8; 4] = [64, 128, 255, 255];
        struct Rect {
            hwnd: HWND,
            fb: PixBuf,
            deadline: Option<std::time::Instant>,
            sampled: Option<u32>,
            painted: bool,
        }
        impl Driver for Rect {
            fn on_create(&mut self, hwnd: HWND) {
                self.hwnd = hwnd;
                let icon = unsafe { GetClassLongPtrW(hwnd, GCLP_HICON) } != 0;
                println!("created, scale {}, class icon from the exe resource: {icon}", scale(hwnd));
            }
            fn on_event(&mut self, ev: Ev) -> bool {
                match ev {
                    Ev::Resize(w, h) => {
                        println!("resize {w}x{h}");
                        self.fb = PixBuf::new(w, h);
                    }
                    Ev::Focus(f) => println!("focus {f}"),
                    Ev::Key { vk: key::ESCAPE, up: false, .. } | Ev::Close => close(self.hwnd),
                    Ev::Timer
                        if self.painted
                            && self.sampled.is_none()
                            && self.deadline.is_some_and(|d| std::time::Instant::now() >= d) =>
                    {
                        let (w, h) = self.fb.dimensions();
                        unsafe {
                            let dc = GetDC(Some(self.hwnd));
                            self.sampled = Some(GetPixel(dc, w as i32 / 2, h as i32 / 2).0);
                            ReleaseDC(Some(self.hwnd), dc);
                            let _ = PostMessageW(Some(self.hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
                        }
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
                    *p = if inside { FILL } else { [24, 24, 28, 255] };
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
        let mut d = Rect { hwnd: HWND::default(), fb: PixBuf::new(1, 1), deadline, sampled: None, painted: false };
        let spec = WindowSpec { title: "rustshot window test".into(), w: 400, h: 300, resizable: true, min: (200, 150), controls: Controls::MinMaxClose };
        let _guard = windows_lock();
        run_window(spec, &mut d).expect("window loop");
        assert!(d.painted);
        if deadline.is_some() {
            // COLORREF is 0x00BBGGRR.
            let want = u32::from_le_bytes([FILL[0], FILL[1], FILL[2], 0]);
            println!("centre pixel {:#08x?}, want {want:#08x}", d.sampled);
            assert_eq!(d.sampled, Some(want));
        }
    }

    /// The pacer: one frame per refresh, started `cost + margin` before
    /// the vblank; a frame already made for a vblank waits for the next;
    /// a slow frame moves the start earlier; a vblank read again later
    /// (another base) neither skips nor repeats a frame.
    #[test]
    fn pacer_plans_one_frame_per_refresh_late_in_it() {
        let period = 1000.0 / 120.0;
        let mut p = Pacer::new();
        let lead = p.cost + PACE_MARGIN_MS;
        // Just after a vblank at 100: wait until the next one less the lead.
        let (wait, target) = p.plan(100.5, 100.0, period);
        assert!((target - (100.0 + period)).abs() < 1e-9, "{target}");
        assert!((wait - (target - lead - 100.5)).abs() < 1e-9, "{wait}");
        // Past the start: now.
        let (wait, target) = p.plan(100.0 + period - 0.5, 100.0, period);
        assert_eq!(wait, 0.0);
        p.made(target, 1.0);
        // Made for that vblank: the next frame is for the one after.
        let (wait, next) = p.plan(100.0 + period - 0.2, 100.0, period);
        assert!((next - (target + period)).abs() < 1e-9);
        assert!(wait > period - lead - 0.5, "{wait}");
        // The same vblank read again from a later base: still the next one.
        let (_, again) = p.plan(100.0 + period - 0.2, 100.0 + 50.0 * period, period);
        assert!((again - next).abs() < 1e-6, "{again} vs {next}");
        // A slow frame: started earlier next time (and a frame never
        // starts before its interval).
        p.made(next, 6.0);
        let (wait, t3) = p.plan(next + 0.1, 100.0, period);
        assert!((wait - (t3 - 6.0 - PACE_MARGIN_MS - (next + 0.1))).abs() < 1e-9, "{wait}");
        p.made(t3, 40.0);
        let (wait, _) = p.plan(t3 + 0.1, 100.0, period);
        assert_eq!(wait, 0.0, "a frame slower than the refresh starts at once");
        // The cost comes down slowly after.
        p.made(t3 + period, 1.0);
        assert!(p.cost > 30.0, "{}", p.cost);
    }

    /// Noted moves: the newest position wins, the times run from the
    /// oldest to the newest, another window's note is not taken.
    #[test]
    fn noted_moves_coalesce() {
        let (a, b) = (HWND(8 as *mut _), HWND(16 as *mut _));
        let t0 = std::time::Instant::now();
        let t1 = t0 + std::time::Duration::from_millis(3);
        NOTED.set(None);
        note_move(a, 1, 2, Some(t0));
        note_move(a, 5, 6, Some(t1));
        let n = NOTED.get().expect("noted");
        assert_eq!((n.x, n.y, n.times), (5, 6, Some((t0, t1))));
        assert!(move_noted());
        struct Rec(Vec<Ev>);
        impl Driver for Rec {
            fn on_create(&mut self, _: HWND) {}
            fn on_event(&mut self, ev: Ev) -> bool {
                self.0.push(ev);
                false
            }
            fn frame(&mut self) -> Option<&mut PixBuf> {
                None
            }
            fn cursor(&self) -> Cursor {
                Cursor::Arrow
            }
        }
        let mut r = Rec(Vec::new());
        assert!(!unsafe { take_move(b, &mut r) }, "not b's");
        assert!(unsafe { take_move(a, &mut r) });
        assert!(matches!(r.0[..], [Ev::Move { x: 5, y: 6 }]));
        assert!(!move_noted() && !unsafe { take_move(a, &mut r) });
        for m in [WM_LBUTTONDOWN, WM_LBUTTONUP, WM_RBUTTONDOWN, WM_MOUSEWHEEL, WM_KEYDOWN, WM_CHAR] {
            assert!(takes_move_first(m), "{m:#x}");
        }
        for m in [WM_PAINT, WM_TIMER, WM_SETCURSOR, WM_MOUSEMOVE] {
            assert!(!takes_move_first(m), "{m:#x}");
        }
    }

    /// What the paced-overlay test saw.
    #[derive(Default)]
    struct PacedSeen {
        evs: Vec<Ev>,
        paints: u32,
    }

    /// A real overlay window (off screen) flooded with moves from another
    /// thread, then a press and a release: the driver gets a handful of
    /// moves (not one per message), the last at the final position and
    /// before the press, and painted frames for them.
    struct Paced {
        seen: std::sync::Arc<std::sync::Mutex<PacedSeen>>,
        helper: Option<std::thread::JoinHandle<()>>,
        hwnd: usize,
        fb: PixBuf,
    }

    impl Driver for Paced {
        fn on_create(&mut self, hwnd: HWND) {
            self.hwnd = hwnd.0 as usize;
            show_at(hwnd, -30000, -30000, 320, 240);
            let raw = hwnd.0 as usize;
            self.helper = Some(std::thread::spawn(move || unsafe {
                let hwnd = HWND(raw as *mut core::ffi::c_void);
                let at = |x: i32, y: i32| LPARAM(((y as isize) << 16) | (x as isize & 0xffff));
                for i in 0..2000 {
                    let _ = PostMessageW(Some(hwnd), WM_MOUSEMOVE, WPARAM(0), at(i % 300, i % 200));
                }
                let _ = PostMessageW(Some(hwnd), WM_LBUTTONDOWN, WPARAM(1), at(7, 9));
                let _ = PostMessageW(Some(hwnd), WM_LBUTTONUP, WPARAM(0), at(7, 9));
                std::thread::sleep(std::time::Duration::from_millis(100));
                let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
            }));
        }
        fn on_event(&mut self, ev: Ev) -> bool {
            let mv = matches!(ev, Ev::Move { .. });
            if matches!(ev, Ev::Timer) {
                return false;
            }
            self.seen.lock().unwrap().evs.push(ev);
            if mv {
                invalidate(HWND(self.hwnd as *mut _));
            }
            false
        }
        fn frame(&mut self) -> Option<&mut PixBuf> {
            self.seen.lock().unwrap().paints += 1;
            Some(&mut self.fb)
        }
        fn cursor(&self) -> Cursor {
            Cursor::Arrow
        }
    }

    #[test]
    fn overlay_draws_moves_once_per_frame_and_keeps_their_order() {
        let _guard = windows_lock();
        let seen = std::sync::Arc::new(std::sync::Mutex::new(PacedSeen::default()));
        let mut d = Paced { seen: seen.clone(), helper: None, hwnd: 0, fb: PixBuf::new(320, 240) };
        std::thread::spawn(move || {
            assert_eq!(run(&mut d), 0);
            d.helper.take().expect("helper").join().expect("helper thread");
        })
        .join()
        .expect("overlay thread");
        let seen = seen.lock().unwrap();
        let moves: Vec<(i32, i32)> = seen.evs.iter().filter_map(|e| if let Ev::Move { x, y } = e { Some((*x, *y)) } else { None }).collect();
        assert!(!moves.is_empty() && moves.len() < 200, "{} moves handled for 2000 messages", moves.len());
        assert_eq!(moves.last(), Some(&(1999 % 300, 1999 % 200)), "the newest position");
        let down = seen.evs.iter().position(|e| matches!(e, Ev::Down { .. })).expect("a press");
        assert!(matches!(seen.evs[down - 1], Ev::Move { x: 199, y: 199 }), "the move before the press: {:?}", &seen.evs[down - 1]);
        assert!(matches!(seen.evs[down..], [Ev::Down { x: 7, y: 9 }, Ev::Up { x: 7, y: 9 }, ..]), "{:?}", &seen.evs[down..]);
        assert!(seen.paints >= 1, "frames painted");
    }

    /// What the pinned-overlay test saw after outside code tried to move it.
    #[derive(Default)]
    struct PinSeen {
        rects: Vec<(&'static str, RECT)>,
        hit: Option<LRESULT>,
    }

    /// Stands in for the editor: shows the overlay at a fixed rect, then a
    /// helper thread plays a window-zoning tool (FancyZones / VirtualZones
    /// style: SetWindowPos / MoveWindow / maximize from another thread)
    /// and a Shift-drag across the client area, and closes it.
    struct Pinned {
        at: RECT,
        seen: std::sync::Arc<std::sync::Mutex<PinSeen>>,
        helper: Option<std::thread::JoinHandle<()>>,
        fb: PixBuf,
    }

    impl Driver for Pinned {
        fn on_create(&mut self, hwnd: HWND) {
            let a = self.at;
            show_at(hwnd, a.left, a.top, a.right - a.left, a.bottom - a.top);
            let raw = hwnd.0 as usize;
            let seen = self.seen.clone();
            self.helper = Some(std::thread::spawn(move || unsafe {
                let hwnd = HWND(raw as *mut core::ffi::c_void);
                let rect = |name: &'static str| {
                    let mut r = RECT::default();
                    let _ = GetWindowRect(hwnd, &mut r);
                    seen.lock().unwrap().rects.push((name, r));
                };
                rect("shown");
                let _ = SetWindowPos(hwnd, None, a.left + 500, a.top + 300, 160, 90, SWP_NOZORDER | SWP_NOACTIVATE);
                rect("SetWindowPos");
                let _ = SetWindowPos(hwnd, None, 0, 0, 100, 50, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE);
                rect("SetWindowPos size only");
                let _ = MoveWindow(hwnd, a.left - 700, a.top + 40, 900, 900, false);
                rect("MoveWindow");
                let _ = SendMessageW(hwnd, WM_SYSCOMMAND, Some(WPARAM(SC_MAXIMIZE as usize)), Some(LPARAM(0)));
                rect("SC_MAXIMIZE");
                let _ = ShowWindow(hwnd, SW_MAXIMIZE);
                rect("SW_MAXIMIZE");
                let _ = ShowWindow(hwnd, SW_RESTORE);
                rect("SW_RESTORE");
                // A hit test anywhere in the window is client: never a caption
                // or border that would start a system move/size loop.
                let (cx, cy) = (a.left + 5, a.top + 5);
                let hit = SendMessageW(hwnd, WM_NCHITTEST, Some(WPARAM(0)), Some(LPARAM(((cy as isize) << 16) | (cx as isize & 0xffff))));
                seen.lock().unwrap().hit = Some(hit);
                // A Shift-drag through the client area.
                let mk = WPARAM(0x0001 | 0x0004); // MK_LBUTTON | MK_SHIFT
                let at = |x: i32, y: i32| LPARAM(((y as isize) << 16) | (x as isize & 0xffff));
                let _ = SendMessageW(hwnd, WM_LBUTTONDOWN, Some(mk), Some(at(10, 10)));
                for i in 1..20 {
                    let _ = SendMessageW(hwnd, WM_MOUSEMOVE, Some(mk), Some(at(10 + i * 8, 10 + i * 3)));
                }
                let _ = SendMessageW(hwnd, WM_LBUTTONUP, Some(WPARAM(0x0004)), Some(at(170, 67)));
                rect("Shift-drag");
                let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
            }));
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

    /// The overlay stays exactly where `show_at` put it whatever another
    /// thread or process asks (window-zoning tools snap the "dragged"
    /// window into a zone: the overlay jumped and half of it was cut off),
    /// hit tests are always client, and a Shift-drag never enters a system
    /// move/size loop. Shown far off screen, so nothing flashes.
    #[test]
    fn overlay_ignores_outside_moves_and_never_enters_move_size() {
        let _guard = windows_lock();
        let at = RECT { left: -30000, top: -30000, right: -30000 + 320, bottom: -30000 + 240 };
        let seen = std::sync::Arc::new(std::sync::Mutex::new(PinSeen::default()));
        let mut d = Pinned { at, seen: seen.clone(), helper: None, fb: PixBuf::new(320, 240) };
        let before = MOVE_SIZE_LOOPS.load(std::sync::atomic::Ordering::SeqCst);
        std::thread::spawn(move || {
            assert_eq!(run(&mut d), 0);
            d.helper.take().expect("helper").join().expect("helper thread");
        })
        .join()
        .expect("overlay thread");
        let seen = seen.lock().unwrap();
        assert_eq!(seen.rects.len(), 8, "{:?}", seen.rects.iter().map(|r| r.0).collect::<Vec<_>>());
        for (name, r) in &seen.rects {
            assert_eq!(
                (r.left, r.top, r.right, r.bottom),
                (at.left, at.top, at.right, at.bottom),
                "{name}: the overlay moved"
            );
        }
        assert_eq!(seen.hit, Some(LRESULT(HTCLIENT as isize)), "hit test");
        assert_eq!(MOVE_SIZE_LOOPS.load(std::sync::atomic::Ordering::SeqCst), before, "no move/size loop");
    }

    #[test]
    fn frame_styles_per_controls() {
        let has = |s: WINDOW_STYLE, f: WINDOW_STYLE| s.0 & f.0 == f.0;
        let (all, ex) = frame_style(Controls::MinMaxClose, true);
        assert!(has(all, WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_MAXIMIZEBOX | WS_THICKFRAME));
        assert_eq!(all, WS_OVERLAPPEDWINDOW, "History / Edit: the standard window");
        assert_eq!(ex, WS_EX_APPWINDOW, "taskbar and Alt+Tab");
        let (fixed, _) = frame_style(Controls::MinClose, false);
        assert!(has(fixed, WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX));
        assert!(!has(fixed, WS_MAXIMIZEBOX) && !has(fixed, WS_THICKFRAME), "Settings: no maximize, no sizing");
        let (dialog, ex) = frame_style(Controls::Close, false);
        assert!(has(dialog, WS_CAPTION | WS_SYSMENU), "close and Alt+F4");
        assert!(!has(dialog, WS_MINIMIZEBOX) && !has(dialog, WS_MAXIMIZEBOX));
        assert_eq!(ex, WS_EX_APPWINDOW);
        // A maximize box needs a window that can be resized.
        assert!(!has(frame_style(Controls::MinMaxClose, false).0, WS_MAXIMIZEBOX));
    }

    #[test]
    fn frame_colours_are_colorrefs_of_the_design_tokens() {
        assert_eq!(colorref([0x1B, 0x1C, 0x20]), 0x0020_1C1B);
        let dark = crate::ui::paint::WTok::of(&crate::theme::DARK).frame();
        assert!(dark.dark);
        assert_eq!(dark.caption, [0x1B, 0x1C, 0x20], "the window surface (--win)");
        // --bd (white at 9 %) over --win.
        assert_eq!(dark.border, [0x30, 0x30, 0x34]);
        let light = crate::ui::paint::WTok::of(&crate::theme::LIGHT).frame();
        assert!(!light.dark);
        assert_eq!(light.caption, [0xFA, 0xFA, 0xFC]);
        assert_eq!(light.border, [0xE6, 0xE6, 0xE8], "--bd (black at 8 %) over --win");
    }

    /// oleacc's STATE_SYSTEM_INVISIBLE (a title bar button not shown).
    const STATE_SYSTEM_INVISIBLE: u32 = 0x8000;

    /// Named WM_NCHITTEST answers.
    type Hits = Vec<(&'static str, isize)>;

    /// WM_GETTITLEBARINFOEX's minimize, maximize and close (state, rect),
    /// and WM_NCHITTEST at the centre of each visible one, on the title,
    /// the left edge and the bottom-right corner.
    unsafe fn caption_probe(hwnd: HWND) -> (Vec<(u32, RECT)>, Hits) {
        unsafe {
            let mut tbi = TITLEBARINFOEX { cbSize: std::mem::size_of::<TITLEBARINFOEX>() as u32, ..Default::default() };
            SendMessageW(hwnd, WM_GETTITLEBARINFOEX, Some(WPARAM(0)), Some(LPARAM(&raw mut tbi as isize)));
            // rgstate/rgrect: 0 title bar, 2 minimize, 3 maximize, 5 close.
            let buttons: Vec<(u32, RECT)> = [2, 3, 5].iter().map(|&i| (tbi.rgstate[i], tbi.rgrect[i])).collect();
            let hit = |x: i32, y: i32| {
                SendMessageW(hwnd, WM_NCHITTEST, Some(WPARAM(0)), Some(LPARAM(((y as isize) << 16) | (x as isize & 0xffff)))).0
            };
            let mid = |r: RECT| ((r.left + r.right) / 2, (r.top + r.bottom) / 2);
            let mut hits = Vec::new();
            for (name, (state, r)) in ["minimize", "maximize", "close"].into_iter().zip(buttons.clone()) {
                if state & STATE_SYSTEM_INVISIBLE == 0 {
                    let (x, y) = mid(r);
                    hits.push((name, hit(x, y)));
                }
            }
            let mut wr = RECT::default();
            let _ = GetWindowRect(hwnd, &mut wr);
            hits.push(("title", hit(wr.left + 120, mid(buttons[2].1).1)));
            hits.push(("left edge", hit(wr.left + 2, (wr.top + wr.bottom) / 2)));
            hits.push(("bottom-right", hit(wr.right - 2, wr.bottom - 2)));
            (buttons, hits)
        }
    }

    /// [`caption_probe`] once the frame has settled: the window shown with
    /// its final rect, DWM flushed, and two reads in a row agreeing (the
    /// first reads after ShowWindow can still see the frame before its
    /// caption metrics and button rects are laid out). `pump` drains this
    /// thread's messages between reads (a window owned by this thread).
    unsafe fn settled_probe(hwnd: HWND, pump: bool) -> (Vec<(u32, RECT)>, Hits) {
        unsafe {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            let rect = || {
                let mut r = RECT::default();
                let _ = GetWindowRect(hwnd, &mut r);
                (r.left, r.top, r.right, r.bottom)
            };
            let mut last = None;
            loop {
                if pump {
                    let mut msg = MSG::default();
                    while PeekMessageW(&mut msg, Some(hwnd), 0, 0, PM_REMOVE).as_bool() {
                        let _ = TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                }
                let _ = windows::Win32::Graphics::Dwm::DwmFlush();
                let now = (rect(), caption_probe(hwnd));
                let laid_out = now.1.0.iter().all(|(state, r)| state & STATE_SYSTEM_INVISIBLE != 0 || r.right > r.left);
                if IsWindowVisible(hwnd).as_bool() && laid_out && last.as_ref() == Some(&now) {
                    return now.1;
                }
                assert!(std::time::Instant::now() < deadline, "the caption never settled: {now:?}");
                last = Some(now);
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        }
    }

    /// Send the caption command `cmd` (WM_SYSCOMMAND, as the buttons,
    /// Win+Down/Up and the system menu do) to a window of another thread
    /// until `done` holds; false if it never does. The window manager can
    /// drop a command silently: an SC_MAXIMIZE sent while the restore from
    /// minimized is still being composed returns having done nothing
    /// (seen on a busy desktop, about 1 run in 20; the same command sent
    /// again a moment later maximizes). So each try waits a while for the
    /// state and then sends the command again, a few times.
    unsafe fn sys_until(hwnd: HWND, cmd: u32, done: &dyn Fn() -> bool) -> bool {
        unsafe {
            for attempt in 1..=4 {
                SendMessageW(hwnd, WM_SYSCOMMAND, Some(WPARAM(cmd as usize)), Some(LPARAM(0)));
                let until = std::time::Instant::now() + std::time::Duration::from_millis(300);
                loop {
                    if done() {
                        if attempt > 1 {
                            eprintln!("WM_SYSCOMMAND {cmd:#x} took effect on try {attempt}");
                        }
                        return true;
                    }
                    if std::time::Instant::now() >= until {
                        break;
                    }
                    let _ = windows::Win32::Graphics::Dwm::DwmFlush();
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
            }
            false
        }
    }

    unsafe extern "system" fn reference_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
    }

    /// [`caption_probe`] on a plain window (DefWindowProc only) with
    /// `style`, shown without activation at `at` on this thread.
    unsafe fn reference_probe(style: u32, at: RECT) -> Hits {
        unsafe {
            let h = HINSTANCE(module_handle().expect("module"));
            let class = w!("rustshot_caption_reference");
            let wc = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(reference_proc),
                hInstance: h,
                lpszClassName: class,
                ..Default::default()
            };
            let _ = RegisterClassExW(&wc); // already registered by an earlier run: fine
            let win = CreateWindowExW(
                WS_EX_APPWINDOW,
                class,
                w!("rustshot caption reference"),
                WINDOW_STYLE(style & !WS_VISIBLE.0),
                at.left,
                at.top,
                at.right - at.left,
                at.bottom - at.top,
                None,
                None,
                Some(h),
                None,
            )
            .expect("reference window");
            let _ = ShowWindow(win, SW_SHOWNOACTIVATE);
            let _ = UpdateWindow(win);
            let (_, hits) = settled_probe(win, true);
            let _ = DestroyWindow(win);
            hits
        }
    }

    /// What the caption test read from a live window.
    #[derive(Debug, Default)]
    struct CaptionSeen {
        /// WM_GETTITLEBARINFOEX: (state, rect) of minimize, maximize, close.
        buttons: Vec<(u32, RECT)>,
        /// WM_NCHITTEST at the centre of each visible button, the title,
        /// and (resizable) the left and bottom-right edges.
        hits: Vec<(&'static str, isize)>,
        /// The same hit tests on a plain window with the same style.
        reference: Vec<(&'static str, isize)>,
        dark_mode: Option<u32>,
        minimized: bool,
        restored: bool,
        maximized: bool,
        unmaximized: bool,
    }

    /// Plays the user on the real caption from a helper thread: reads the
    /// title bar buttons, hit-tests them, minimizes / restores / maximizes
    /// with WM_SYSCOMMAND (what the buttons, Win+Down/Up and the system
    /// menu send), then the close button's WM_CLOSE.
    struct Captioned {
        hwnd: HWND,
        fb: PixBuf,
        seen: std::sync::Arc<std::sync::Mutex<CaptionSeen>>,
        helper: Option<std::thread::JoinHandle<()>>,
        closes: u32,
    }

    impl Driver for Captioned {
        fn on_create(&mut self, hwnd: HWND) {
            self.hwnd = hwnd;
            let raw = hwnd.0 as usize;
            let seen = self.seen.clone();
            self.helper = Some(std::thread::spawn(move || unsafe {
                use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE};
                let hwnd = HWND(raw as *mut core::ffi::c_void);
                // Physical coordinates, as the window's thread uses them.
                SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
                // Shown and themed (the loop syncs the frame before ShowWindow).
                while !IsWindowVisible(hwnd).as_bool() {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                let mut s = CaptionSeen::default();
                (s.buttons, s.hits) = settled_probe(hwnd, false);
                // The same frame on a plain DefWindowProc window: the
                // reference for what Windows itself answers.
                let mut wr = RECT::default();
                let _ = GetWindowRect(hwnd, &mut wr);
                s.reference = reference_probe(GetWindowLongW(hwnd, GWL_STYLE) as u32, wr);
                let mut dark = 0u32;
                if DwmGetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, (&raw mut dark).cast(), 4).is_ok() {
                    s.dark_mode = Some(dark);
                }
                let iconic = || IsIconic(hwnd).as_bool();
                let zoomed = || IsZoomed(hwnd).as_bool();
                s.minimized = sys_until(hwnd, SC_MINIMIZE, &iconic);
                s.restored = sys_until(hwnd, SC_RESTORE, &|| !iconic());
                s.maximized = sys_until(hwnd, SC_MAXIMIZE, &zoomed);
                s.unmaximized = sys_until(hwnd, SC_RESTORE, &|| !zoomed());
                *seen.lock().unwrap() = s;
                // The close button / Alt+F4.
                let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
            }));
        }
        fn on_event(&mut self, ev: Ev) -> bool {
            match ev {
                Ev::Resize(w, h) => self.fb = PixBuf::new(w.max(1), h.max(1)),
                Ev::Close => {
                    self.closes += 1;
                    close(self.hwnd);
                }
                _ => {}
            }
            false
        }
        fn frame(&mut self) -> Option<&mut PixBuf> {
            Some(&mut self.fb)
        }
        fn cursor(&self) -> Cursor {
            Cursor::Arrow
        }
        fn frame_colors(&self) -> Option<FrameColors> {
            Some(FrameColors { dark: true, caption: [0x1B, 0x1C, 0x20], border: [0x30, 0x31, 0x34] })
        }
    }

    fn run_captioned(controls: Controls, resizable: bool) -> (CaptionSeen, u32) {
        let seen = std::sync::Arc::new(std::sync::Mutex::new(CaptionSeen::default()));
        let s2 = seen.clone();
        let closes = std::thread::spawn(move || {
            let mut d = Captioned { hwnd: HWND::default(), fb: PixBuf::new(1, 1), seen: s2, helper: None, closes: 0 };
            let spec = WindowSpec { title: "rustshot caption test".into(), w: 400, h: 300, resizable, min: (200, 150), controls };
            run_window(spec, &mut d).expect("window loop");
            d.helper.take().expect("helper").join().expect("helper thread");
            assert!(unsafe { !IsWindow(Some(d.hwnd)).as_bool() }, "closed by the close button");
            d.closes
        })
        .join()
        .expect("window thread");
        let s = std::mem::take(&mut *seen.lock().unwrap());
        (s, closes)
    }

    /// The real caption buttons per window kind: resizable windows get
    /// minimize, maximize/restore and close and sizing edges; fixed-size
    /// ones minimize and close (maximize disabled); dialogs close only.
    /// Every hit test answers what Windows answers for a plain window with
    /// the same frame (the Snap Layouts flyout, Aero snap and dragging are
    /// the OS's own), minimize/restore/maximize work, the close button
    /// reaches the driver once, and the dark title bar is applied.
    #[test]
    fn run_window_caption_buttons() {
        let _guard = windows_lock();
        const INVISIBLE: u32 = STATE_SYSTEM_INVISIBLE;
        const UNAVAILABLE: u32 = 0x0001; // STATE_SYSTEM_UNAVAILABLE
        let hits = |s: &CaptionSeen| s.hits.iter().map(|&(n, h)| (n, h as u32)).collect::<Vec<_>>();
        // Windows 11 answers HTCAPTION over minimize and maximize even on a
        // plain DefWindowProc window (DWM tracks those two buttons itself,
        // Snap Layouts included); older builds answer the button.
        let button = |h: u32, b: u32| h == b || h == HTCAPTION;

        let (s, closes) = run_captioned(Controls::MinMaxClose, true);
        println!("resizable: {s:?}");
        assert_eq!(closes, 1);
        assert_eq!(s.hits, s.reference, "hit tests as on a plain window with this frame");
        let states: Vec<u32> = s.buttons.iter().map(|b| b.0 & (INVISIBLE | UNAVAILABLE)).collect();
        assert_eq!(states, [0, 0, 0], "minimize, maximize, close all enabled");
        let h = hits(&s);
        assert_eq!(h.iter().map(|x| x.0).collect::<Vec<_>>(), ["minimize", "maximize", "close", "title", "left edge", "bottom-right"]);
        assert!(button(h[0].1, HTMINBUTTON) && button(h[1].1, HTMAXBUTTON), "{h:?}");
        assert_eq!(&h[2..], [("close", HTCLOSE), ("title", HTCAPTION), ("left edge", HTLEFT), ("bottom-right", HTBOTTOMRIGHT)]);
        let w = |r: &RECT| r.right - r.left;
        assert!(s.buttons.iter().all(|b| w(&b.1) > 0), "visible buttons have rects");
        assert!(s.minimized && s.restored, "minimize and restore");
        assert!(s.maximized && s.unmaximized, "maximize and restore");
        if let Some(d) = s.dark_mode {
            assert_eq!(d, 1, "immersive dark mode applied");
        }

        let (s, closes) = run_captioned(Controls::MinClose, false);
        println!("fixed: {s:?}");
        assert_eq!(closes, 1);
        assert_eq!(s.hits, s.reference, "hit tests as on a plain window with this frame");
        assert_eq!(s.buttons[0].0 & (INVISIBLE | UNAVAILABLE), 0, "minimize enabled");
        assert_ne!(s.buttons[1].0 & (INVISIBLE | UNAVAILABLE), 0, "maximize disabled or hidden");
        assert_eq!(s.buttons[2].0 & (INVISIBLE | UNAVAILABLE), 0, "close enabled");
        let h = hits(&s);
        assert!(h[0].0 == "minimize" && button(h[0].1, HTMINBUTTON), "{h:?}");
        assert!(h.contains(&("close", HTCLOSE)) && h.contains(&("title", HTCAPTION)), "{h:?}");
        assert!(!h.contains(&("left edge", HTLEFT)) && !h.contains(&("bottom-right", HTBOTTOMRIGHT)), "no sizing: {h:?}");
        assert!(s.minimized && s.restored, "minimize and restore");
        assert!(!s.maximized, "no maximize (Win+Up, double-click on the title)");

        let (s, closes) = run_captioned(Controls::Close, false);
        println!("dialog: {s:?}");
        assert_eq!(closes, 1);
        assert_eq!(s.hits, s.reference, "hit tests as on a plain window with this frame");
        assert_ne!(s.buttons[0].0 & INVISIBLE, 0, "no minimize");
        assert_ne!(s.buttons[1].0 & INVISIBLE, 0, "no maximize");
        assert_eq!(s.buttons[2].0 & (INVISIBLE | UNAVAILABLE), 0, "close");
        assert_eq!(hits(&s)[0], ("close", HTCLOSE));
        assert!(!s.maximized);
    }

    #[link(name = "user32")]
    unsafe extern "system" {
        fn PrintWindow(hwnd: HWND, hdc: windows::Win32::Graphics::Gdi::HDC, flags: u32) -> i32;
    }

    #[link(name = "ntdll")]
    unsafe extern "system" {
        fn RtlGetVersion(info: *mut windows::Win32::System::SystemInformation::OSVERSIONINFOW) -> i32;
    }

    /// The Windows build number (RtlGetVersion: no manifest lies).
    fn os_build() -> u32 {
        let mut v = windows::Win32::System::SystemInformation::OSVERSIONINFOW {
            dwOSVersionInfoSize: std::mem::size_of::<windows::Win32::System::SystemInformation::OSVERSIONINFOW>() as u32,
            ..Default::default()
        };
        if unsafe { RtlGetVersion(&mut v) } == 0 { v.dwBuildNumber } else { 0 }
    }

    /// The whole window as DWM composes it (title bar included), RGBA,
    /// from its own redirection surface: nothing else on screen is read.
    unsafe fn print_window(hwnd: HWND) -> PixBuf {
        unsafe {
            let mut wr = RECT::default();
            let _ = GetWindowRect(hwnd, &mut wr);
            let (w, h) = (wr.right - wr.left, wr.bottom - wr.top);
            let mut bi: BITMAPINFO = std::mem::zeroed();
            bi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            (bi.bmiHeader.biWidth, bi.bmiHeader.biHeight) = (w, -h);
            (bi.bmiHeader.biPlanes, bi.bmiHeader.biBitCount) = (1, 32);
            let dc = CreateCompatibleDC(None);
            let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
            let bmp = CreateDIBSection(Some(dc), &bi, DIB_RGB_COLORS, &mut bits, None, 0).expect("DIB");
            let old = SelectObject(dc, bmp.into());
            const PW_RENDERFULLCONTENT: u32 = 2;
            PrintWindow(hwnd, dc, PW_RENDERFULLCONTENT);
            let _ = GdiFlush();
            let mut img = PixBuf::new(w as u32, h as u32);
            img.as_raw_mut().copy_from_slice(std::slice::from_raw_parts(bits as *const u8, (w * h * 4) as usize));
            swap_rb(img.as_raw_mut());
            img.as_raw_mut().as_chunks_mut::<4>().0.iter_mut().for_each(|p| p[3] = 255);
            SelectObject(dc, old);
            let _ = DeleteObject(bmp.into());
            let _ = DeleteDC(dc);
            img
        }
    }

    /// The title bar pixel right of the title text and left of the
    /// buttons, if the bar is laid out inside the print.
    fn caption_sample(img: &PixBuf, bar: RECT) -> Option<[u8; 4]> {
        let (x, y) = (bar.left + (bar.right - bar.left) * 3 / 5, (bar.top + bar.bottom) / 2);
        let inside = bar.right > bar.left && bar.bottom > bar.top && x >= 0 && y >= 0;
        (inside && (x as u32) < img.width() && (y as u32) < img.height()).then(|| img.get_pixel(x as u32, y as u32))
    }

    fn near(p: [u8; 4], want: [u8; 3]) -> bool {
        p.iter().zip(want).all(|(&a, b)| a.abs_diff(b) <= 3)
    }

    /// Fills its client area with the "design surface" and reports the
    /// title bar colours for `dark` (flipped from the helper thread).
    struct Themed {
        hwnd: HWND,
        fb: PixBuf,
        dark: std::sync::Arc<std::sync::atomic::AtomicBool>,
        helper: Option<std::thread::JoinHandle<Vec<(bool, PixBuf, RECT)>>>,
    }

    impl Themed {
        fn colors(dark: bool) -> FrameColors {
            crate::ui::paint::WTok::of(if dark { &crate::theme::DARK } else { &crate::theme::LIGHT }).frame()
        }
    }

    impl Driver for Themed {
        fn on_create(&mut self, hwnd: HWND) {
            self.hwnd = hwnd;
            let raw = hwnd.0 as usize;
            let dark = self.dark.clone();
            self.helper = Some(std::thread::spawn(move || unsafe {
                let hwnd = HWND(raw as *mut core::ffi::c_void);
                SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
                // Shown, laid out and composed before the first print.
                settled_probe(hwnd, false);
                // DWMWA_CAPTION_COLOR arrived with Windows 11 (build 22000).
                let tinted = os_build() >= 22000;
                let shot = || {
                    let mut wr = RECT::default();
                    let _ = GetWindowRect(hwnd, &mut wr);
                    let mut tbi = TITLEBARINFOEX { cbSize: std::mem::size_of::<TITLEBARINFOEX>() as u32, ..Default::default() };
                    SendMessageW(hwnd, WM_GETTITLEBARINFOEX, Some(WPARAM(0)), Some(LPARAM(&raw mut tbi as isize)));
                    // Cropped to the visible frame (no invisible resize borders);
                    // the title bar in the crop's coordinates.
                    use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};
                    let mut vis = wr;
                    let _ = DwmGetWindowAttribute(hwnd, DWMWA_EXTENDED_FRAME_BOUNDS, (&raw mut vis).cast(), 16);
                    let full = print_window(hwnd);
                    let (ox, oy) = ((vis.left - wr.left).max(0) as u32, (vis.top - wr.top).max(0) as u32);
                    let (cw, ch) = ((vis.right - vis.left) as u32, (vis.bottom - vis.top) as u32);
                    let mut img = PixBuf::new(cw.min(full.width() - ox), ch.min(full.height() - oy));
                    let (iw, fw) = (img.width() as usize * 4, full.width() as usize * 4);
                    for (y, row) in img.as_raw_mut().chunks_exact_mut(iw).enumerate() {
                        let at = (y + oy as usize) * fw + ox as usize * 4;
                        row.copy_from_slice(&full.as_raw()[at..at + iw]);
                    }
                    let t = tbi.rcTitleBar;
                    (img, RECT { left: t.left - vis.left, top: t.top - vis.top, right: t.right - vis.left, bottom: t.bottom - vis.top })
                };
                let mut shots = Vec::new();
                for d in [true, false, true] {
                    dark.store(d, std::sync::atomic::Ordering::SeqCst);
                    // A theme switch repaints; the frame follows on that paint.
                    let _ = InvalidateRect(Some(hwnd), None, false);
                    // DWM applies the colour asynchronously: print until the
                    // title bar shows it (or, where DWM keeps its own colour,
                    // until two prints agree), within a deadline.
                    let want = Themed::colors(d).caption;
                    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
                    let mut last: Option<[u8; 4]> = None;
                    let got = loop {
                        let _ = windows::Win32::Graphics::Dwm::DwmFlush();
                        let (img, bar) = shot();
                        let p = caption_sample(&img, bar);
                        let done = if tinted { p.is_some_and(|p| near(p, want)) } else { p.is_some() && p == last };
                        if done || std::time::Instant::now() >= deadline {
                            break (img, bar);
                        }
                        last = p;
                        std::thread::sleep(std::time::Duration::from_millis(30));
                    };
                    shots.push((d, got.0, got.1));
                }
                let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
                shots
            }));
        }
        fn on_event(&mut self, ev: Ev) -> bool {
            match ev {
                Ev::Resize(w, h) => self.fb = PixBuf::new(w.max(1), h.max(1)),
                Ev::Close => close(self.hwnd),
                _ => {}
            }
            false
        }
        fn frame(&mut self) -> Option<&mut PixBuf> {
            let [r, g, b] = Self::colors(self.dark.load(std::sync::atomic::Ordering::SeqCst)).caption;
            self.fb.as_raw_mut().as_chunks_mut::<4>().0.iter_mut().for_each(|p| *p = [r, g, b, 255]);
            Some(&mut self.fb)
        }
        fn cursor(&self) -> Cursor {
            Cursor::Arrow
        }
        fn frame_colors(&self) -> Option<FrameColors> {
            Some(Self::colors(self.dark.load(std::sync::atomic::Ordering::SeqCst)))
        }
    }

    /// The OS title bar takes the design's surface colour, dark and light,
    /// and follows a theme switch while the window is open (Windows 11:
    /// DWMWA_CAPTION_COLOR; older builds keep their own caption, so the
    /// colour check is skipped there). Writes `caption-*.png` (the whole
    /// window, frame included) under `RUSTSHOT_PREVIEW_DIR`.
    #[test]
    fn run_window_caption_takes_the_design_colours() {
        let _guard = windows_lock();
        let shots = std::thread::spawn(|| {
            let dark = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
            let mut d = Themed { hwnd: HWND::default(), fb: PixBuf::new(1, 1), dark, helper: None };
            let spec = WindowSpec {
                title: "Rustshot Settings".into(),
                w: 480,
                h: 200,
                resizable: false,
                min: (480, 200),
                controls: Controls::MinClose,
            };
            run_window(spec, &mut d).expect("window loop");
            d.helper.take().expect("helper").join().expect("helper thread")
        })
        .join()
        .expect("window thread");
        let build = os_build();
        for (i, (dark, img, bar)) in shots.iter().enumerate() {
            let name = format!("caption-{}{}.png", if *dark { "dark" } else { "light" }, if i == 2 { "-again" } else { "" });
            crate::ui::preview::save(&name, img);
            assert!(bar.bottom > bar.top && bar.right > bar.left, "title bar rect {bar:?}");
            let p = caption_sample(img, *bar).expect("title bar inside the print");
            let want = Themed::colors(*dark).caption;
            println!("{name}: title bar {p:?}, want {want:?}, build {build}");
            if build >= 22000 {
                assert!(near(p, want), "{name}: title bar {p:?}, want the design surface {want:?}");
            }
        }
    }
}
