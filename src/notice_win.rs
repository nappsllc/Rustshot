//! Windows: the notice in a layered, top-most tool window that never
//! activates and is excluded from capture. A note without buttons lets
//! clicks through (`WS_EX_TRANSPARENT`); one with buttons (after a
//! recording, the thumbnail card) takes them, and holds while the pointer
//! is on it. Dragging the thumbnail out drops its file (`SHDoDragDrop`
//! with the shell's data object for it: `CF_HDROP` and the shell's own
//! formats). It lives on the UI thread and fades on its own timer.

use super::{act_at, act_layout, hold, note_size, opacity, place_note, render_note, ActLayout, Next, Note, NoteAct, FADE_IN};
use crate::objects::Pt;
use crate::rec_ui::Dib;
use crate::theme::Theme;
use std::cell::RefCell;
use std::sync::Mutex;
use std::time::Instant;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT};
use windows::Win32::UI::WindowsAndMessaging::*;

const CLASS: PCWSTR = w!("rustshot_notice");
const TIMER: usize = 1;
const TICK_MS: u32 = 16;
const WM_MOUSELEAVE: u32 = 0x02A3;
/// `WM_MOUSEMOVE`'s wParam: the left button is down.
const MK_LBUTTON: usize = 0x0001;
/// An action finished on another thread (an upload): its note is in [`LATER`].
const WM_LATER: u32 = WM_APP + 1;

struct Up {
    note: Note,
    th: Theme,
    s: f32,
    pos: (i32, i32),
    since: Instant,
    /// Opacity of the frame on screen (None: none drawn yet).
    k: Option<f32>,
    /// The buttons (a note with actions) and the one under the pointer.
    layout: Option<ActLayout>,
    hover: Option<NoteAct>,
    /// The hover changed since the last frame.
    dirty: bool,
    /// The left button went down here (window px): a click, or a drag
    /// out of a thumbnail once it moves.
    press: Option<Pt>,
    /// Its age while held (see [`hold`]).
    frozen: Option<std::time::Duration>,
}

struct Win {
    hwnd: HWND,
    dib: Dib,
    up: Option<Up>,
    tracking: bool,
}

thread_local! {
    static WIN: RefCell<Option<Win>> = const { RefCell::new(None) };
}

/// A note an action's thread finished with, for the UI thread.
static LATER: Mutex<Option<Note>> = Mutex::new(None);

fn lp_pt(lp: LPARAM) -> Pt {
    Pt::new((lp.0 & 0xFFFF) as u16 as i16 as f32, ((lp.0 >> 16) & 0xFFFF) as u16 as i16 as f32)
}

/// The note up has buttons: it takes clicks.
fn clickable() -> bool {
    WIN.with(|w| w.try_borrow().ok().and_then(|w| w.as_ref().and_then(|w| w.up.as_ref().map(|u| u.layout.is_some()))).unwrap_or(false))
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_TIMER => {
                tick();
                LRESULT(0)
            }
            WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
            WM_NCHITTEST if !clickable() => LRESULT(HTTRANSPARENT as isize),
            WM_MOUSEMOVE | WM_MOUSELEAVE => {
                let p = (msg == WM_MOUSEMOVE).then(|| lp_pt(lp));
                hover(hwnd, p);
                if let Some(p) = p
                    && wp.0 & MK_LBUTTON != 0
                {
                    drag_out(hwnd, p);
                }
                LRESULT(0)
            }
            WM_LBUTTONDOWN => {
                // Captured: the release comes here wherever it happens, so
                // a press never holds the note for good.
                SetCapture(hwnd);
                set_press(Some(lp_pt(lp)));
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                let p = lp_pt(lp);
                let press = set_press(None);
                let _ = ReleaseCapture();
                // A click only after a press on the same button here.
                if let Some(q) = press
                    && same_target(q, p)
                {
                    click(p);
                }
                LRESULT(0)
            }
            WM_CAPTURECHANGED => {
                set_press(None);
                LRESULT(0)
            }
            WM_LATER => {
                let note = LATER.lock().unwrap_or_else(|e| e.into_inner()).take();
                if let Some(n) = note
                    && !replace(n.clone())
                {
                    crate::tray::notify(&n.text);
                }
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

/// The pointer moved over the note (`None`: it left).
fn hover(hwnd: HWND, p: Option<Pt>) {
    WIN.with(|w| {
        let mut w = w.borrow_mut();
        let Some(win) = w.as_mut() else { return };
        if p.is_some() && !win.tracking {
            let mut t = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
            win.tracking = unsafe { TrackMouseEvent(&mut t) }.is_ok();
        }
        if p.is_none() {
            win.tracking = false;
        }
        let Some(up) = win.up.as_mut() else { return };
        let Some(l) = &up.layout else { return };
        let h = p.and_then(|p| act_at(l, p));
        if h != up.hover {
            up.hover = h;
            up.dirty = true;
        }
    });
    tick();
}

/// A click on a button: carry it out; the note says how it went (or goes).
fn click(p: Pt) {
    let job = WIN.with(|w| {
        let w = w.borrow();
        let up = w.as_ref()?.up.as_ref()?;
        let a = act_at(up.layout.as_ref()?, p)?;
        Some((a, up.note.clone(), up.th, w.as_ref()?.hwnd.0 as isize))
    });
    let Some((a, note, th, hwnd)) = job else { return };
    let next = super::run_note_action(&note, a, th, move |n| {
        *LATER.lock().unwrap_or_else(|e| e.into_inner()) = Some(n);
        let _ = unsafe { PostMessageW(Some(HWND(hwnd as *mut _)), WM_LATER, WPARAM(0), LPARAM(0)) };
    });
    match next {
        Next::Keep => {}
        Next::Show(n) => {
            replace(n);
        }
        // Open / Show in folder, Pin, Close: done with the note.
        Next::Hide => hide(),
    }
}

/// Points `a` and `b` (window px) are on the same button of the note up.
fn same_target(a: Pt, b: Pt) -> bool {
    WIN.with(|w| {
        let w = w.borrow();
        let Some(l) = w.as_ref().and_then(|w| w.up.as_ref()).and_then(|u| u.layout.as_ref()) else { return false };
        act_at(l, a).is_some() && act_at(l, a) == act_at(l, b)
    })
}

/// Set (or clear) the press on the note; the one before.
fn set_press(p: Option<Pt>) -> Option<Pt> {
    WIN.with(|w| {
        let mut w = w.borrow_mut();
        let up = w.as_mut()?.up.as_mut()?;
        std::mem::replace(&mut up.press, p)
    })
}

/// The pointer moved to `p` with the left button down: once it is past
/// the system's drag distance from a press on a thumbnail (not on a
/// button), the file is dragged out; the card goes once it is dropped.
fn drag_out(hwnd: HWND, p: Pt) {
    let (dx, dy) = unsafe { (GetSystemMetrics(SM_CXDRAG).max(1) as f32, GetSystemMetrics(SM_CYDRAG).max(1) as f32) };
    let note = WIN.with(|w| {
        let w = w.borrow();
        let up = w.as_ref()?.up.as_ref()?;
        let press = up.press?;
        up.note.thumb.as_ref()?;
        let on = act_at(up.layout.as_ref()?, press)?;
        (on == NoteAct::Card && ((p.x - press.x).abs() > dx || (p.y - press.y).abs() > dy)).then(|| up.note.clone())
    });
    let Some(note) = note else { return };
    set_press(None);
    let Some(file) = super::drag_file(&note) else { return };
    match drag_file_out(hwnd, &file) {
        // Dropped: this card goes (not a newer note that replaced it meanwhile).
        Ok(true) => {
            if WIN.with(|w| w.borrow().as_ref().and_then(|w| w.up.as_ref()).is_some_and(|u| u.note == note)) {
                hide();
            }
        }
        Ok(false) => {}
        Err(e) => eprintln!("rustshot: drag {}: {e}", file.display()),
    }
}

/// Drag `file` out (a modal loop until it is dropped or called off):
/// true when something took it.
fn drag_file_out(hwnd: HWND, file: &std::path::Path) -> windows::core::Result<bool> {
    use windows::Win32::System::Ole::{DROPEFFECT_COPY, DROPEFFECT_NONE};
    use windows::Win32::UI::Shell::SHDoDragDrop;
    let data = data_object(file)?;
    // No drop source of ours: the shell's default one.
    let effect = unsafe { SHDoDragDrop(Some(hwnd), &data, None, DROPEFFECT_COPY)? };
    Ok(effect != DROPEFFECT_NONE)
}

/// Run `init` (OLE's initialisation) once on this thread.
fn ole_once(init: impl FnOnce() -> windows::core::Result<()>) {
    thread_local! {
        static DONE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }
    if !DONE.with(|d| d.replace(true))
        && let Err(e) = init()
    {
        eprintln!("rustshot: OLE: {e}");
    }
}

/// The shell's data object for `file` (what Explorer drags: `CF_HDROP`
/// and its own formats).
fn data_object(file: &std::path::Path) -> windows::core::Result<windows::Win32::System::Com::IDataObject> {
    use windows::Win32::System::Ole::OleInitialize;
    use windows::Win32::UI::Shell::{IShellItem, SHCreateItemFromParsingName, BHID_DataObject};
    let path: Vec<u16> = file.as_os_str().to_string_lossy().encode_utf16().chain(Some(0)).collect();
    unsafe {
        // Drag and drop needs OLE on this thread (S_FALSE: it was already).
        ole_once(|| OleInitialize(None));
        let item: IShellItem = SHCreateItemFromParsingName(PCWSTR(path.as_ptr()), None)?;
        item.BindToHandler(None, &BHID_DataObject)
    }
}

fn create() -> Option<HWND> {
    static CLASS_OK: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let ok = *CLASS_OK.get_or_init(|| unsafe {
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

/// Clicks through (a plain note) or not (one with buttons).
fn set_click_through(hwnd: HWND, through: bool) {
    unsafe {
        let ex = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
        let want = if through { ex | WS_EX_TRANSPARENT.0 } else { ex & !WS_EX_TRANSPARENT.0 };
        if want != ex {
            SetWindowLongW(hwnd, GWL_EXSTYLE, want as i32);
        }
    }
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

fn up_for(note: Note, th: Theme, s: f32, pos: (i32, i32), since: Instant) -> Up {
    let layout = (!note.actions.is_empty()).then(|| act_layout(&note, s));
    Up { note, th, s, pos, since, k: None, layout, hover: None, dirty: false, press: None, frozen: None }
}

pub fn show(note: Note, at: (i32, i32), th: Theme) -> bool {
    let (work, s) = monitor_at(at);
    let pos = place_note(work, &note, note_size(&note, s));
    let shown = WIN.with(|w| {
        let mut w = w.borrow_mut();
        if w.is_none() {
            *w = create().map(|hwnd| Win { hwnd, dib: Dib::default(), up: None, tracking: false });
        }
        let Some(win) = w.as_mut() else { return false };
        set_click_through(win.hwnd, note.actions.is_empty());
        win.up = Some(up_for(note, th, s, pos, Instant::now()));
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
        let Some(up) = win.up.as_ref() else { return false };
        let (work, s) = monitor_at(up.pos);
        let now = Instant::now();
        let pos = place_note(work, &note, note_size(&note, s));
        set_click_through(win.hwnd, note.actions.is_empty());
        // On screen already: no second fade-in.
        win.up = Some(up_for(note, up.th, s, pos, now.checked_sub(FADE_IN).unwrap_or(now)));
        unsafe {
            let _ = SetTimer(Some(win.hwnd), TIMER, TICK_MS, None);
        }
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
    // The picture of a thumbnail is not kept once it is down.
    super::forget_picture();
    unsafe {
        let _ = KillTimer(Some(win.hwnd), TIMER);
        let _ = ShowWindow(win.hwnd, SW_HIDE);
    }
}

/// Draw the next frame of the fade, or take the note down once it is over.
fn tick() {
    WIN.with(|w| {
        let Ok(mut w) = w.try_borrow_mut() else { return };
        let Some(win) = w.as_mut() else { return };
        let Some(up) = win.up.as_mut() else { return };
        // Held while the pointer is on its buttons (or the card), or it
        // is pressed.
        let held = up.hover.is_some() || up.press.is_some();
        up.since = hold(up.since, Instant::now(), up.note.ttl, held, &mut up.frozen);
        let Some(k) = opacity(up.since.elapsed(), up.note.ttl) else {
            take_down(win);
            return;
        };
        if !up.dirty && up.k.is_some_and(|old| (old - k).abs() < 0.004) {
            return; // holding: no new frame
        }
        let first = up.k.is_none();
        up.k = Some(k);
        up.dirty = false;
        let (bw, bh, px) = render_note(&up.note, &up.th, up.s, k, up.hover);
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

    fn ex_style(hwnd: HWND) -> u32 {
        unsafe { GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 }
    }

    #[test]
    fn the_notice_window_never_takes_focus_or_clicks_and_is_not_captured() {
        let note = |t: &str| Note { ttl: std::time::Duration::from_millis(300), ..Note::plain(t, ToastKind::Info) };
        assert!(!replace(note("x")), "nothing up yet");
        assert!(show(note("Uploading\u{2026}"), (0, 0), crate::theme::DARK));
        let hwnd = WIN.with(|w| w.borrow().as_ref().unwrap().hwnd);
        unsafe {
            let ex = ex_style(hwnd);
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

    /// The thumbnail card: bottom-right of the work area, clickable, its
    /// buttons drawn once the pointer is on it (and the card held), Pin
    /// pins the capture where it was and takes the card down.
    #[test]
    fn the_thumbnail_window() {
        let img = std::sync::Arc::new(crate::pin::tests::sample_shot(320, 180));
        let n = super::super::thumbnail(&[crate::export::Done::Copied], false, img, (12, 34)).unwrap();
        assert!(show(n.clone(), (0, 0), crate::theme::DARK));
        let hwnd = WIN.with(|w| w.borrow().as_ref().unwrap().hwnd);
        assert_eq!(ex_style(hwnd) & WS_EX_TRANSPARENT.0, 0, "it takes clicks");
        let (work, s) = monitor_at((0, 0));
        let mut r = windows::Win32::Foundation::RECT::default();
        unsafe { GetWindowRect(hwnd, &mut r).unwrap() };
        assert_eq!((r.right, r.bottom), (work.0 + work.2 as i32, work.1 + work.3 as i32), "bottom-right");
        let l = act_layout(&n, s);
        let card = l.panel;
        hover(hwnd, Some(Pt::new(card.x + 4.0, card.y + 4.0)));
        assert_eq!(WIN.with(|w| w.borrow().as_ref().unwrap().up.as_ref().unwrap().hover), Some(NoteAct::Card));
        let frozen = WIN.with(|w| w.borrow().as_ref().unwrap().up.as_ref().unwrap().frozen);
        assert!(frozen.is_some(), "held while hovered");
        let _ = crate::pin::take_opened();
        let pin = l.buttons.iter().find(|b| b.0 == NoteAct::Pin).unwrap().1;
        let save = l.buttons.iter().find(|b| b.0 == NoteAct::ThumbSave).unwrap().1;
        let lp = |r: crate::objects::FRect| LPARAM(((r.y + r.h / 2.0) as isize) << 16 | (r.x + r.w / 2.0) as isize);
        let pressed = || WIN.with(|w| w.borrow().as_ref().unwrap().up.as_ref().unwrap().press);
        unsafe {
            // A press, then the capture is lost (released elsewhere, another
            // window took it): no longer pressed, the clock goes on.
            SendMessageW(hwnd, WM_LBUTTONDOWN, Some(WPARAM(1)), Some(lp(pin)));
            assert!(pressed().is_some());
            SendMessageW(hwnd, WM_CAPTURECHANGED, None, None);
            assert!(pressed().is_none(), "the capture went: the press too");
            // Pressed on Pin, released on Save: neither acts.
            SendMessageW(hwnd, WM_LBUTTONDOWN, Some(WPARAM(1)), Some(lp(pin)));
            SendMessageW(hwnd, WM_LBUTTONUP, None, Some(lp(save)));
            assert!(pressed().is_none());
        }
        assert!(crate::pin::take_opened().is_empty(), "no click across buttons");
        assert!(WIN.with(|w| w.borrow().as_ref().unwrap().up.is_some()), "the card is still up");
        click(Pt::new(pin.x + pin.w / 2.0, pin.y + pin.h / 2.0));
        assert_eq!(crate::pin::take_opened(), [((320, 180), (12, 34))]);
        assert!(WIN.with(|w| w.borrow().as_ref().unwrap().up.is_none()), "the card went");
        assert!(!unsafe { IsWindowVisible(hwnd) }.as_bool());
    }

    /// What dragging the card out hands over: the shell's data object for
    /// the file, with CF_HDROP.
    #[test]
    fn the_drag_carries_the_file() {
        use windows::Win32::System::Com::{DVASPECT_CONTENT, FORMATETC, TYMED_HGLOBAL};
        let f = std::env::temp_dir().join(format!("rustshot-drag-{}.png", std::process::id()));
        std::fs::write(&f, crate::pixbuf::PixBuf::from_pixel(2, 2, [1, 2, 3, 255]).to_png().unwrap()).unwrap();
        let data = data_object(&f).expect("a data object");
        let fmt = FORMATETC { cfFormat: 15, ptd: std::ptr::null_mut(), dwAspect: DVASPECT_CONTENT.0, lindex: -1, tymed: TYMED_HGLOBAL.0 as u32 };
        assert!(unsafe { data.QueryGetData(&fmt) }.is_ok(), "CF_HDROP");
        let _ = std::fs::remove_file(&f);
    }

    /// A note with buttons takes clicks (not click-through) and stays 6 s;
    /// a plain one after it lets them through again. A click on Copy puts
    /// the file on the clipboard (tests: no clipboard) and says so.
    #[test]
    fn a_note_with_buttons_takes_clicks() {
        let path = std::path::PathBuf::from("C:\\shots\\rustshot-2026-10-10-2131.gif");
        let n = super::super::recording_saved(&path, std::time::Duration::from_secs(42), 1_400_000, &[], true);
        assert_eq!(n.ttl, super::super::ACTION_TTL);
        assert!(show(n.clone(), (0, 0), crate::theme::LIGHT));
        let hwnd = WIN.with(|w| w.borrow().as_ref().unwrap().hwnd);
        assert_eq!(ex_style(hwnd) & WS_EX_TRANSPARENT.0, 0, "clickable");
        assert_eq!(ex_style(hwnd) & WS_EX_NOACTIVATE.0, WS_EX_NOACTIVATE.0, "still never activates");
        let l = act_layout(&n, WIN.with(|w| w.borrow().as_ref().unwrap().up.as_ref().unwrap().s));
        let copy = l.buttons.iter().find(|b| b.0 == NoteAct::Copy).unwrap().1;
        let c = Pt::new(copy.x + copy.w / 2.0, copy.y + copy.h / 2.0);
        hover(hwnd, Some(c));
        assert_eq!(WIN.with(|w| w.borrow().as_ref().unwrap().up.as_ref().unwrap().hover), Some(NoteAct::Copy));
        click(c);
        let shown = WIN.with(|w| w.borrow().as_ref().unwrap().up.as_ref().map(|u| u.note.text.clone()));
        assert_eq!(shown.as_deref(), Some("Copied to clipboard"));
        assert_ne!(ex_style(hwnd) & WS_EX_TRANSPARENT.0, 0, "plain again: click-through");
        hide();
    }
}
