//! Windows: `WH_MOUSE_LL` / `WH_KEYBOARD_LL` hooks on a thread of their
//! own (a low-level hook runs on the thread that set it, which must pump
//! messages), installed for the recording and removed with it, whatever
//! happens (a start that times out included). The hook procedures only
//! look at the moment of the event and queue it; a second thread files it
//! into the [`Marks`]. Nothing is logged.
//!
//! - Modifiers are read when the key goes down (`GetAsyncKeyState`), never
//!   derived from earlier events: a key-up lost to the lock screen or a UAC
//!   prompt cannot leave Win or Ctrl stuck.
//! - AltGr (layouts with it: German, Polish, French, …) reports a synthetic
//!   left Ctrl with the right Alt; that pair is no modifier, so typed @, €,
//!   { or ą never read as Ctrl + Alt shortcuts.
//! - With "All keys" a password field is never shown: focus changes
//!   (`EVENT_OBJECT_FOCUS`, the foreground window) mark the focus unknown
//!   at once, and so do Tab, Enter and clicks (the focus is about to move);
//!   a checker thread then asks what has the focus (a classic Edit with
//!   `ES_PASSWORD`, UI Automation's IsPassword). A key typed while the
//!   answer is unknown or "password" is not shown (fail closed). Without
//!   "All keys" nothing is asked.

use super::{Button, Held, Hooks, Marks, STALE};
use crate::chan::{self, Receiver, RecvTimeoutError, Sender};
use anyhow::{anyhow, Context, Result};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{HANDLE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::StationsAndDesktops::{CloseDesktop, GetUserObjectInformationW, OpenInputDesktop, DESKTOP_CONTROL_FLAGS, DESKTOP_READOBJECTS, UOI_NAME};
use windows::Win32::System::Threading::{GetCurrentThread, GetCurrentThreadId, SetThreadPriority, THREAD_PRIORITY_TIME_CRITICAL};
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, CUIAutomation, HWINEVENTHOOK, IUIAutomation};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, GetKeyboardLayout, VkKeyScanExW, HKL};
use windows::Win32::UI::WindowsAndMessaging::*;

/// What a hook saw, with the moment it saw it.
enum Ev {
    Click(i32, i32, Button, Instant),
    Key { vk: u32, held: Held, secret: bool, at: Instant },
}

/// Where the hook procedures put their events (one recording at a time).
static SINK: Mutex<Option<Sender<Ev>>> = Mutex::new(None);
/// The recording shows every key (the password check is on).
static ALL_KEYS: AtomicBool = AtomicBool::new(false);

fn send(e: Ev) {
    if let Some(tx) = SINK.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        let _ = tx.send(e);
    }
}

// ---------------------------------------------------------------------------
// Modifiers at the moment of the key
// ---------------------------------------------------------------------------

const VK_LSHIFT: u32 = 0xA0;
const VK_RSHIFT: u32 = 0xA1;
const VK_LCONTROL: u32 = 0xA2;
const VK_RCONTROL: u32 = 0xA3;
const VK_LMENU: u32 = 0xA4;
const VK_RMENU: u32 = 0xA5;
const VK_LWIN: u32 = 0x5B;
const VK_RWIN: u32 = 0x5C;

/// The chord modifiers from `down` (a key held now?) on a layout where
/// right Alt is AltGr (`altgr`, asked only while right Alt is down).
/// AltGr comes with a synthetic left Ctrl: with right Alt down on such a
/// layout, neither counts (a real left Ctrl held with AltGr is lost, which
/// only ever shows less).
fn held_from(down: &dyn Fn(u32) -> bool, altgr: &dyn Fn() -> bool) -> Held {
    let ralt = down(VK_RMENU);
    let gr = ralt && altgr();
    Held {
        ctrl: down(VK_RCONTROL) || (down(VK_LCONTROL) && !gr),
        alt: down(VK_LMENU) || (ralt && !gr),
        shift: down(VK_LSHIFT) || down(VK_RSHIFT),
        win: down(VK_LWIN) || down(VK_RWIN),
    }
}

fn async_down(vk: u32) -> bool {
    // SAFETY: a plain query; 0 off the input desktop (fails closed).
    unsafe { GetAsyncKeyState(vk as i32) as u16 & 0x8000 != 0 }
}

/// Characters that only AltGr types on the layouts that have it.
const ALTGR_CHARS: &str = "@€{}[]\\|~ąęłśżźćńó߀µ²³";

/// The foreground window's layout has AltGr (cached per layout).
fn altgr_layout() -> bool {
    static CACHE: Mutex<Vec<(isize, bool)>> = Mutex::new(Vec::new());
    // SAFETY: plain queries of the foreground window and its layout.
    let hkl = unsafe {
        let fg = GetForegroundWindow();
        GetKeyboardLayout(GetWindowThreadProcessId(fg, None))
    };
    let mut c = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(&(_, v)) = c.iter().find(|(h, _)| *h == hkl.0 as isize) {
        return v;
    }
    let v = has_altgr(hkl);
    c.push((hkl.0 as isize, v));
    v
}

fn has_altgr(hkl: HKL) -> bool {
    ALTGR_CHARS.encode_utf16().any(|ch| {
        // SAFETY: a plain lookup in the layout's tables (no key state).
        let r = unsafe { VkKeyScanExW(ch, hkl) };
        r != -1 && (r as u16 >> 8) & 6 == 6
    })
}

// ---------------------------------------------------------------------------
// The password check
// ---------------------------------------------------------------------------

/// What has the focus: 0 unknown, 1 not a password field, 2 a password field.
static PW: AtomicU8 = AtomicU8::new(0);
/// Bumped at each focus change; an answer for an older one is dropped.
static GEN: AtomicU32 = AtomicU32::new(0);
/// The checker's requests: (generation, ask no earlier than).
static CHECK: Mutex<Option<Sender<(u32, Instant)>>> = Mutex::new(None);

/// After Tab, Enter or a click the focus may move a moment later: the
/// answer is asked once the focus event came, or after this.
const MOVE_WAIT: Duration = Duration::from_millis(300);

/// The focus is unknown from now on; ask after `wait`.
fn invalidate(wait: Duration) {
    if !ALL_KEYS.load(Ordering::SeqCst) {
        return;
    }
    let g = GEN.fetch_add(1, Ordering::SeqCst).wrapping_add(1);
    PW.store(0, Ordering::SeqCst);
    if let Some(tx) = CHECK.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        let _ = tx.send((g, Instant::now() + wait));
    }
}

/// A key typed now must not show: unknown or a password field.
fn secret_now() -> bool {
    ALL_KEYS.load(Ordering::SeqCst) && PW.load(Ordering::SeqCst) != 1
}

/// Answer the requests until the channel closes.
fn checker(rx: &Receiver<(u32, Instant)>, ask: &dyn Fn() -> bool) {
    let mut pending: Option<(u32, Instant)> = None;
    loop {
        let msg = match pending {
            None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
            Some((_, at)) => rx.recv_timeout(at.saturating_duration_since(Instant::now())),
        };
        match msg {
            // The newest request wins (a focus event moves it sooner).
            Ok(r) => pending = Some(r),
            Err(RecvTimeoutError::Timeout) => {
                if let Some((g, _)) = pending.take()
                    && GEN.load(Ordering::SeqCst) == g
                {
                    let v = if ask() { 2 } else { 1 };
                    // Kept only if the focus did not move meanwhile
                    // (`invalidate` bumps the generation first).
                    if PW.compare_exchange(0, v, Ordering::SeqCst, Ordering::SeqCst).is_ok() && GEN.load(Ordering::SeqCst) != g {
                        PW.store(0, Ordering::SeqCst);
                    }
                }
            }
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

unsafe extern "system" fn focus_proc(_h: HWINEVENTHOOK, _event: u32, _hwnd: HWND, _obj: i32, _child: i32, _thread: u32, _time: u32) {
    invalidate(Duration::ZERO);
}

// ---------------------------------------------------------------------------
// The hook procedures
// ---------------------------------------------------------------------------

/// The key event `vk` (message `msg`) at `at`, the key state read through
/// `down`: `None` for releases and modifiers (they only qualify others).
fn key_event(vk: u32, msg: u32, down: &dyn Fn(u32) -> bool, altgr: &dyn Fn() -> bool, at: Instant) -> Option<Ev> {
    if !matches!(msg, WM_KEYDOWN | WM_SYSKEYDOWN) {
        return None;
    }
    if matches!(vk, 0x09 | 0x0D) {
        // Tab, Enter: the focus is about to move (this key still shows
        // by what had the focus).
        let secret = secret_now();
        invalidate(MOVE_WAIT);
        return Some(Ev::Key { vk, held: held_from(down, altgr), secret, at });
    }
    if super::is_modifier(vk) {
        return None;
    }
    Some(Ev::Key { vk, held: held_from(down, altgr), secret: secret_now(), at })
}

unsafe extern "system" fn mouse_proc(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 && lp.0 != 0 {
        // SAFETY: for HC_ACTION, lParam points to the hook's MSLLHOOKSTRUCT.
        let m = unsafe { &*(lp.0 as *const MSLLHOOKSTRUCT) };
        let b = match wp.0 as u32 {
            WM_LBUTTONDOWN => Some(Button::Left),
            WM_RBUTTONDOWN => Some(Button::Right),
            WM_MBUTTONDOWN | WM_XBUTTONDOWN => None,
            _ => return unsafe { CallNextHookEx(None, code, wp, lp) },
        };
        // A click may move the focus.
        invalidate(MOVE_WAIT);
        if let Some(b) = b {
            send(Ev::Click(m.pt.x, m.pt.y, b, Instant::now()));
        }
    }
    unsafe { CallNextHookEx(None, code, wp, lp) }
}

unsafe extern "system" fn key_proc(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 && lp.0 != 0 {
        // SAFETY: for HC_ACTION, lParam points to the hook's KBDLLHOOKSTRUCT.
        let k = unsafe { &*(lp.0 as *const KBDLLHOOKSTRUCT) };
        if let Some(e) = key_event(k.vkCode, wp.0 as u32, &async_down, &altgr_layout, Instant::now()) {
            send(e);
        }
    }
    unsafe { CallNextHookEx(None, code, wp, lp) }
}

// ---------------------------------------------------------------------------
// Starting and stopping
// ---------------------------------------------------------------------------

/// Shared with the hook thread: its id once known, and "stop".
#[derive(Default)]
struct Ctl {
    tid: AtomicU32,
    quit: AtomicBool,
}

/// The threads; dropping it removes the hooks and ends them all (it owns
/// them from the moment they start, so a failed start cleans up too).
struct WinHooks {
    ctl: Arc<Ctl>,
    hook: Option<std::thread::JoinHandle<()>>,
    filer: Option<std::thread::JoinHandle<()>>,
    checker: Option<std::thread::JoinHandle<()>>,
}

impl Hooks for WinHooks {}

impl Drop for WinHooks {
    fn drop(&mut self) {
        self.ctl.quit.store(true, Ordering::SeqCst);
        if let Some(h) = self.hook.take() {
            // WM_QUIT until it is gone (a post before its queue exists is
            // lost; the thread also checks `quit` itself).
            for _ in 0..300 {
                if h.is_finished() {
                    break;
                }
                let tid = self.ctl.tid.load(Ordering::SeqCst);
                if tid != 0 {
                    // SAFETY: a thread id of ours; WM_QUIT ends its loop.
                    unsafe {
                        let _ = PostThreadMessageW(tid, WM_QUIT, WPARAM(0), LPARAM(0));
                    }
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            if h.is_finished() {
                let _ = h.join();
            }
        }
        // The channels go: the filer and the checker end.
        *SINK.lock().unwrap_or_else(|e| e.into_inner()) = None;
        *CHECK.lock().unwrap_or_else(|e| e.into_inner()) = None;
        ALL_KEYS.store(false, Ordering::SeqCst);
        for t in [self.filer.take(), self.checker.take()].into_iter().flatten() {
            let _ = t.join();
        }
    }
}

pub fn start(marks: Arc<Marks>) -> Result<Box<dyn Hooks>> {
    let opts = marks.opts;
    let all = opts.keys && opts.all_keys;
    let ctl = Arc::new(Ctl::default());
    let mut guard = WinHooks { ctl: ctl.clone(), hook: None, filer: None, checker: None };
    let (tx, rx) = chan::channel();
    *SINK.lock().unwrap_or_else(|e| e.into_inner()) = Some(tx);
    guard.filer = Some(std::thread::Builder::new().name("rec-marks".into()).spawn(move || file_events(&marks, &rx)).context("start the keystroke thread")?);
    if all {
        PW.store(0, Ordering::SeqCst);
        let (ctx, crx) = chan::channel();
        *CHECK.lock().unwrap_or_else(|e| e.into_inner()) = Some(ctx);
        ALL_KEYS.store(true, Ordering::SeqCst);
        guard.checker = Some(
            std::thread::Builder::new()
                .name("rec-marks-focus".into())
                .spawn(move || {
                    crate::rec::win::com_init();
                    // SAFETY: a COM object of this thread.
                    let uia: Option<IUIAutomation> = unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok() };
                    checker(&crx, &|| password_focused(uia.as_ref()));
                })
                .context("start the focus thread")?,
        );
        invalidate(Duration::ZERO);
    }
    let (ready_tx, ready_rx) = chan::channel();
    guard.hook = Some(
        std::thread::Builder::new()
            .name("rec-hooks".into())
            .spawn(move || hook_thread(&ctl, opts.clicks, opts.keys, all, &ready_tx))
            .context("start the hook thread")?,
    );
    match ready_rx.recv_timeout(Duration::from_secs(2)) {
        Ok(true) => Ok(Box::new(guard)),
        Ok(false) => Err(anyhow!("cannot install the input hooks")),
        Err(_) => Err(anyhow!("the hooks did not start")),
    }
}

/// The hook thread: hooks in, messages pumped until WM_QUIT (or `quit`),
/// hooks out, whichever way it ends.
fn hook_thread(ctl: &Ctl, clicks: bool, keys: bool, focus: bool, ready: &Sender<bool>) {
    // SAFETY: plain calls on this thread; every hook set here is removed
    // before it returns.
    unsafe {
        ctl.tid.store(GetCurrentThreadId(), Ordering::SeqCst);
        // Its message queue exists from here: a WM_QUIT is no longer lost.
        let mut msg = MSG::default();
        let _ = PeekMessageW(&mut msg, None, WM_USER, WM_USER, PM_NOREMOVE);
        if ctl.quit.load(Ordering::SeqCst) {
            return;
        }
        // A starved hook is dropped by Windows (LowLevelHooksTimeout).
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_TIME_CRITICAL);
        let inst = GetModuleHandleW(None).ok().map(Into::into);
        let mouse = if clicks || focus { SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), inst, 0).ok() } else { None };
        let kb = if keys { SetWindowsHookExW(WH_KEYBOARD_LL, Some(key_proc), inst, 0).ok() } else { None };
        let mut events: Vec<HWINEVENTHOOK> = Vec::new();
        if focus {
            for e in [EVENT_OBJECT_FOCUS, EVENT_SYSTEM_FOREGROUND] {
                let h = SetWinEventHook(e, e, None, Some(focus_proc), 0, 0, WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS);
                if !h.is_invalid() {
                    events.push(h);
                }
            }
        }
        let ok = (mouse.is_some() || !(clicks || focus)) && (kb.is_some() || !keys) && (!focus || events.len() == 2);
        if ok && !ctl.quit.load(Ordering::SeqCst) {
            let _ = ready.send(true);
            while !ctl.quit.load(Ordering::SeqCst) && GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        } else {
            let _ = ready.send(false);
        }
        for h in [mouse, kb].into_iter().flatten() {
            let _ = UnhookWindowsHookEx(h);
        }
        for h in events {
            let _ = UnhookWinEvent(h);
        }
    }
}

/// File the hooks' events into `marks` until the sink is dropped; a key
/// that waited too long is dropped, not drawn late.
fn file_events(marks: &Marks, rx: &Receiver<Ev>) {
    while let Ok(e) = rx.recv() {
        match e {
            Ev::Click(x, y, b, at) => marks.click(x, y, b, at),
            Ev::Key { vk, held, secret, at } => {
                if at.elapsed() <= STALE {
                    marks.key(vk, held, secret, at);
                }
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

/// The input desktop is not the user's ("Default"): Winlogon (the lock
/// screen, Ctrl+Alt+Del) or a UAC prompt; one that cannot be opened counts.
pub fn secure_desktop() -> bool {
    // SAFETY: a desktop handle opened and closed here.
    unsafe {
        let Ok(d) = OpenInputDesktop(DESKTOP_CONTROL_FLAGS(0), false, DESKTOP_READOBJECTS) else { return true };
        let mut name = [0u16; 64];
        let mut len = 0u32;
        let ok = GetUserObjectInformationW(HANDLE(d.0), UOI_NAME, Some(name.as_mut_ptr().cast()), (name.len() * 2) as u32, Some(&mut len)).is_ok();
        let _ = CloseDesktop(d);
        let n = name.iter().position(|&c| c == 0).unwrap_or(name.len());
        !ok || !String::from_utf16_lossy(&name[..n]).eq_ignore_ascii_case("Default")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rec::marks::MarkOpts;

    /// A keyboard: which keys are down.
    /// The tests share the hooks' globals: one at a time.
    fn globals() -> std::sync::MutexGuard<'static, ()> {
        static T: Mutex<()> = Mutex::new(());
        T.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn kb(keys: &[u32]) -> impl Fn(u32) -> bool + '_ {
        move |vk| keys.contains(&vk)
    }

    fn chord(keys: &[u32], vk: u32, altgr: bool, all: bool) -> Option<String> {
        let Some(Ev::Key { held, .. }) = key_event(vk, WM_KEYDOWN, &kb(keys), &|| altgr, Instant::now()) else { return None };
        super::super::chord_text(held, vk, all)
    }

    /// AltGr (a synthetic left Ctrl with the right Alt) on German, Polish
    /// and French layouts types characters, not shortcuts.
    #[test]
    fn altgr_is_no_chord() {
        let _g = globals();
        let altgr = [VK_LCONTROL, VK_RMENU];
        // German: AltGr+Q = @, AltGr+E = €, AltGr+7 = {.
        for vk in [0x51, 0x45, 0x37, 0xDB] {
            assert_eq!(chord(&altgr, vk, true, false), None, "German AltGr {vk:#x}");
        }
        // Polish (programmer): AltGr+A/E/L/S/Z = ą ę ł ś ż; Shift too.
        for vk in [0x41, 0x45, 0x4C, 0x53, 0x5A] {
            assert_eq!(chord(&altgr, vk, true, false), None, "Polish AltGr {vk:#x}");
            assert_eq!(chord(&[VK_LCONTROL, VK_RMENU, VK_LSHIFT], vk, true, false), None, "Polish Shift+AltGr {vk:#x}");
        }
        // French AZERTY: AltGr+0 = @, AltGr+3 = #.
        assert_eq!(chord(&altgr, 0x30, true, false), None);
        // With every key shown: the key alone, never "Ctrl + Alt".
        assert_eq!(chord(&altgr, 0x51, true, true).as_deref(), Some("Q"));
        // Real chords on those layouts still show.
        assert_eq!(chord(&[VK_LCONTROL, VK_LMENU], 0x51, true, false).as_deref(), Some("Ctrl + Alt + Q"));
        assert_eq!(chord(&[VK_RCONTROL], 0x53, true, false).as_deref(), Some("Ctrl + S"));
        // A layout without AltGr (US): right Alt is Alt.
        assert_eq!(chord(&[VK_RMENU], 0x46, false, false).as_deref(), Some("Alt + F"));
        // The synthetic Ctrl itself and the Alt are no events.
        assert!(key_event(VK_LCONTROL, WM_KEYDOWN, &kb(&[]), &|| true, Instant::now()).is_none());
        assert!(key_event(VK_RMENU, WM_SYSKEYDOWN, &kb(&altgr), &|| true, Instant::now()).is_none());
    }

    /// Modifiers are what is held at the key, not what earlier events
    /// said: a Win whose key-up was lost (Win+L) does not stick.
    #[test]
    fn modifiers_are_read_at_the_key() {
        let _g = globals();
        assert_eq!(chord(&[VK_LWIN], 0x4C, false, false).as_deref(), Some(format!("{} + L", crate::keymap::META_NAME)).as_deref());
        // After unlocking nothing is held: typing is typing.
        assert_eq!(chord(&[], 0x48, false, false), None);
        assert!(key_event(0x48, WM_KEYUP, &kb(&[VK_LWIN]), &|| false, Instant::now()).is_none(), "releases are no events");
    }

    #[test]
    fn layouts_with_altgr_are_told() {
        use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyboardLayoutList, LoadKeyboardLayoutW, UnloadKeyboardLayout};
        // SAFETY: plain layout loads. A layout loaded here stays in the
        // user's session (and in the tray's layout switcher) until logoff,
        // so what the test adds it unloads again, before any assertion.
        unsafe {
            let mut before = vec![HKL::default(); 64];
            let n = GetKeyboardLayoutList(Some(&mut before)) as usize;
            before.truncate(n);
            let us = LoadKeyboardLayoutW(windows::core::w!("00000409"), Default::default()).ok();
            let de = LoadKeyboardLayoutW(windows::core::w!("00000407"), Default::default()).ok();
            let altgr = (us.map(has_altgr), de.map(has_altgr));
            for hkl in [us, de].into_iter().flatten() {
                if !before.contains(&hkl) {
                    let _ = UnloadKeyboardLayout(hkl);
                }
            }
            if let (Some(us), Some(de)) = altgr {
                assert!(!us, "US");
                assert!(de, "German");
            }
        }
    }

    /// The hook procedures, called as Windows would (no hook is
    /// installed): what they queue reaches the marks through the filer.
    #[test]
    fn hook_procedures_feed_the_marks() {
        let _g = globals();
        let marks = Arc::new(Marks::new(MarkOpts { clicks: true, keys: true, all_keys: false }));
        let (tx, rx) = chan::channel();
        *SINK.lock().unwrap() = Some(tx);
        let m2 = marks.clone();
        let filer = std::thread::spawn(move || file_events(&m2, &rx));
        let now = Instant::now();
        let ctrl = |vk: u32| vk == VK_LCONTROL;
        send(key_event(0x53, WM_KEYDOWN, &ctrl, &|| false, now).unwrap());
        // A stale key is dropped, not drawn late.
        send(key_event(0x55, WM_KEYDOWN, &ctrl, &|| false, now - Duration::from_secs(2)).unwrap());
        let m = MSLLHOOKSTRUCT { pt: windows::Win32::Foundation::POINT { x: 12, y: 34 }, ..Default::default() };
        unsafe { mouse_proc(HC_ACTION as i32, WPARAM(WM_RBUTTONDOWN as usize), LPARAM(&m as *const _ as isize)) };
        *SINK.lock().unwrap() = None;
        filer.join().unwrap();
        let s = marks.scene(Instant::now());
        assert_eq!(s.pill.map(|p| p.0).as_deref(), Some("Ctrl + S"));
        assert_eq!(s.rings.len(), 1);
        assert_eq!((s.rings[0].0.x, s.rings[0].0.y, s.rings[0].0.button), (12, 34, Button::Right));
    }

    /// All keys: unknown focus is secret; Tab marks it unknown until the
    /// checker answers for the new focus; an answer for an old focus is
    /// dropped.
    #[test]
    fn the_password_flag_fails_closed() {
        let _g = globals();
        ALL_KEYS.store(true, Ordering::SeqCst);
        let (tx, rx) = chan::channel();
        *CHECK.lock().unwrap() = Some(tx);
        let answer = Arc::new(AtomicBool::new(false));
        let a2 = answer.clone();
        let t = std::thread::spawn(move || checker(&rx, &|| a2.load(Ordering::SeqCst)));
        invalidate(Duration::ZERO);
        assert!(secret_now(), "unknown at first");
        let wait = |want: u8| {
            let t0 = Instant::now();
            while PW.load(Ordering::SeqCst) != want && t0.elapsed() < Duration::from_secs(3) {
                std::thread::sleep(Duration::from_millis(5));
            }
            PW.load(Ordering::SeqCst)
        };
        assert_eq!(wait(1), 1, "a plain field");
        assert!(!secret_now());
        // Tab: unknown at once, and the key typed after it is secret
        // until the answer for the new focus (a password field) comes.
        answer.store(true, Ordering::SeqCst);
        let Some(Ev::Key { secret, .. }) = key_event(0x09, WM_KEYDOWN, &kb(&[]), &|| false, Instant::now()) else { panic!() };
        assert!(!secret, "Tab itself is judged by the field it left");
        let Some(Ev::Key { secret, .. }) = key_event(0x41, WM_KEYDOWN, &kb(&[]), &|| false, Instant::now()) else { panic!() };
        assert!(secret, "right after Tab: unknown, not shown");
        assert_eq!(wait(2), 2, "the password field");
        assert!(secret_now());
        *CHECK.lock().unwrap() = None;
        ALL_KEYS.store(false, Ordering::SeqCst);
        t.join().unwrap();
        assert!(!secret_now(), "without All keys nothing is asked");
    }

    /// A start that fails or times out leaves no hook thread behind.
    #[test]
    fn a_dropped_guard_ends_the_hook_thread() {
        let _g = globals();
        let ctl = Arc::new(Ctl::default());
        let (tx, rx) = chan::channel();
        let c2 = ctl.clone();
        // No hooks asked: the thread only pumps.
        let h = std::thread::spawn(move || hook_thread(&c2, false, false, false, &tx));
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)), Ok(true));
        let guard = WinHooks { ctl, hook: Some(h), filer: None, checker: None };
        let t0 = Instant::now();
        drop(guard);
        assert!(t0.elapsed() < Duration::from_secs(2), "it ended");
        // Asked to quit before it started: it never pumps.
        let ctl = Arc::new(Ctl::default());
        ctl.quit.store(true, Ordering::SeqCst);
        let (tx, _rx) = chan::channel();
        let c2 = ctl.clone();
        std::thread::spawn(move || hook_thread(&c2, true, true, false, &tx)).join().unwrap();
    }

    #[test]
    fn this_desktop_is_not_the_secure_one() {
        assert!(!secure_desktop());
    }
}
