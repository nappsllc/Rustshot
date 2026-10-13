//! Windows: `WH_MOUSE_LL` / `WH_KEYBOARD_LL` hooks on a thread of their
//! own (a low-level hook runs on the thread that set it, which must pump
//! messages), installed for the recording and removed with it. The hook
//! procedures only queue the event and return; a second thread files it
//! into the [`Marks`], asking UI Automation first whether the focused
//! element is a password field (then the key is never shown). Nothing is
//! logged.

use super::{Button, Hooks, Marks};
use crate::chan::{self, Receiver, Sender};
use anyhow::{anyhow, Context, Result};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Accessibility::{CUIAutomation, IUIAutomation};
use windows::Win32::UI::WindowsAndMessaging::*;

/// What a hook saw.
enum Ev {
    Click(i32, i32, Button),
    Key(u32, bool),
}

/// Where the hook procedures put their events (one recording at a time).
static SINK: Mutex<Option<Sender<Ev>>> = Mutex::new(None);

fn send(e: Ev) {
    if let Some(tx) = SINK.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        let _ = tx.send(e);
    }
}

unsafe extern "system" fn mouse_proc(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 && lp.0 != 0 {
        // SAFETY: for HC_ACTION, lParam points to the hook's MSLLHOOKSTRUCT.
        let m = unsafe { &*(lp.0 as *const MSLLHOOKSTRUCT) };
        match wp.0 as u32 {
            WM_LBUTTONDOWN => send(Ev::Click(m.pt.x, m.pt.y, Button::Left)),
            WM_RBUTTONDOWN => send(Ev::Click(m.pt.x, m.pt.y, Button::Right)),
            _ => {}
        }
    }
    unsafe { CallNextHookEx(None, code, wp, lp) }
}

unsafe extern "system" fn key_proc(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 && lp.0 != 0 {
        // SAFETY: for HC_ACTION, lParam points to the hook's KBDLLHOOKSTRUCT.
        let k = unsafe { &*(lp.0 as *const KBDLLHOOKSTRUCT) };
        match wp.0 as u32 {
            WM_KEYDOWN | WM_SYSKEYDOWN => send(Ev::Key(k.vkCode, true)),
            WM_KEYUP | WM_SYSKEYUP => send(Ev::Key(k.vkCode, false)),
            _ => {}
        }
    }
    unsafe { CallNextHookEx(None, code, wp, lp) }
}

/// The two threads; dropping it removes the hooks and ends both.
struct WinHooks {
    hook_tid: u32,
    hook: Option<std::thread::JoinHandle<()>>,
    filer: Option<std::thread::JoinHandle<()>>,
}

impl Hooks for WinHooks {}

impl Drop for WinHooks {
    fn drop(&mut self) {
        // SAFETY: a thread id of ours; WM_QUIT ends its message loop.
        unsafe {
            let _ = PostThreadMessageW(self.hook_tid, WM_QUIT, WPARAM(0), LPARAM(0));
        }
        if let Some(h) = self.hook.take() {
            let _ = h.join();
        }
        // The sink goes: the filer's receiver ends.
        *SINK.lock().unwrap_or_else(|e| e.into_inner()) = None;
        if let Some(f) = self.filer.take() {
            let _ = f.join();
        }
    }
}

pub fn start(marks: Arc<Marks>) -> Result<Box<dyn Hooks>> {
    let (tx, rx) = chan::channel();
    *SINK.lock().unwrap_or_else(|e| e.into_inner()) = Some(tx);
    let opts = marks.opts;
    let filer = std::thread::Builder::new().name("rec-marks".into()).spawn(move || {
        crate::rec::win::com_init();
        let uia: Option<IUIAutomation> = unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok() };
        file_events(&marks, &rx, &|| password_focused(uia.as_ref()))
    }).context("start the keystroke thread")?;
    let (ready_tx, ready_rx) = chan::channel();
    let hook = std::thread::Builder::new()
        .name("rec-hooks".into())
        .spawn(move || {
            // SAFETY: plain hook calls on this thread, which pumps messages
            // until WM_QUIT and removes its hooks before it ends.
            unsafe {
                let inst = GetModuleHandleW(None).ok().map(Into::into);
                let mouse = if opts.clicks { SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), inst, 0).ok() } else { None };
                let keys = if opts.keys { SetWindowsHookExW(WH_KEYBOARD_LL, Some(key_proc), inst, 0).ok() } else { None };
                let ok = (mouse.is_some() || !opts.clicks) && (keys.is_some() || !opts.keys);
                let _ = ready_tx.send((GetCurrentThreadId(), ok));
                let mut msg = MSG::default();
                while ok && GetMessageW(&mut msg, None, 0, 0).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
                for h in [mouse, keys].into_iter().flatten() {
                    let _ = UnhookWindowsHookEx(h);
                }
            }
        })
        .context("start the hook thread")?;
    let (tid, ok) = ready_rx.recv_timeout(Duration::from_secs(2)).map_err(|_| anyhow!("the hooks did not start"))?;
    let guard = WinHooks { hook_tid: tid, hook: Some(hook), filer: Some(filer) };
    if !ok {
        drop(guard);
        return Err(anyhow!("cannot install the input hooks"));
    }
    Ok(Box::new(guard))
}

/// File the hooks' events into `marks` until the sink is dropped.
fn file_events(marks: &Marks, rx: &Receiver<Ev>, is_secret: &dyn Fn() -> bool) {
    let mut checked: Option<(Instant, bool)> = None;
    while let Ok(e) = rx.recv() {
        let now = Instant::now();
        match e {
            Ev::Click(x, y, b) => marks.click(x, y, b, now),
            Ev::Key(vk, down) => {
                // Asked only for keys that could show (not for modifiers,
                // nor for releases), and then at most every 150 ms.
                let secret = down
                    && super::chord_text(super::Held { ctrl: true, ..Default::default() }, vk, true).is_some()
                    && match checked {
                        Some((t, s)) if now.saturating_duration_since(t) < Duration::from_millis(150) => s,
                        _ => {
                            let s = is_secret();
                            checked = Some((now, s));
                            s
                        }
                    };
                marks.key(vk, down, secret, now);
            }
        }
    }
}

/// A password field has the keyboard focus: a classic Edit control with
/// `ES_PASSWORD`, or what UI Automation says of the focused element.
fn password_focused(uia: Option<&IUIAutomation>) -> bool {
    // SAFETY: plain queries of the focus.
    unsafe {
        let mut gti = GUITHREADINFO { cbSize: std::mem::size_of::<GUITHREADINFO>() as u32, ..Default::default() };
        if GetGUIThreadInfo(0, &mut gti).is_ok() && !gti.hwndFocus.is_invalid() && classic_password(gti.hwndFocus) {
            return true;
        }
        uia.and_then(|u| u.GetFocusedElement().ok()).and_then(|el| el.CurrentIsPassword().ok()).is_some_and(|b| b.as_bool())
    }
}

/// An Edit control with the password style.
unsafe fn classic_password(h: HWND) -> bool {
    const ES_PASSWORD: i32 = 0x20;
    let mut name = [0u16; 16];
    let n = unsafe { GetClassNameW(h, &mut name) }.max(0) as usize;
    let class = String::from_utf16_lossy(&name[..n.min(name.len())]);
    class.eq_ignore_ascii_case("Edit") && unsafe { GetWindowLongW(h, GWL_STYLE) } & ES_PASSWORD != 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rec::marks::MarkOpts;

    /// The hook procedures, called as Windows would (no hook is
    /// installed): what they queue reaches the marks through the filer.
    #[test]
    fn hook_procedures_feed_the_marks() {
        let marks = Arc::new(Marks::new(MarkOpts { clicks: true, keys: true, all_keys: false }));
        let (tx, rx) = chan::channel();
        *SINK.lock().unwrap() = Some(tx);
        let m2 = marks.clone();
        let filer = std::thread::spawn(move || file_events(&m2, &rx, &|| false));
        let key = |vk: u32, msg: u32| {
            let k = KBDLLHOOKSTRUCT { vkCode: vk, ..Default::default() };
            unsafe { key_proc(HC_ACTION as i32, WPARAM(msg as usize), LPARAM(&k as *const _ as isize)) };
        };
        key(0xA2, WM_KEYDOWN);
        key(0x53, WM_KEYDOWN);
        key(0x53, WM_KEYUP);
        key(0xA2, WM_KEYUP);
        let m = MSLLHOOKSTRUCT { pt: windows::Win32::Foundation::POINT { x: 12, y: 34 }, ..Default::default() };
        unsafe { mouse_proc(HC_ACTION as i32, WPARAM(WM_RBUTTONDOWN as usize), LPARAM(&m as *const _ as isize)) };
        *SINK.lock().unwrap() = None;
        filer.join().unwrap();
        let s = marks.scene(Instant::now());
        assert_eq!(s.pill.map(|p| p.0).as_deref(), Some("Ctrl + S"));
        assert_eq!(s.rings.len(), 1);
        assert_eq!((s.rings[0].0.x, s.rings[0].0.y, s.rings[0].0.button), (12, 34, Button::Right));
    }
}
