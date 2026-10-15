//! Windows notification-area icon. It lives on the hidden top-level `rustshot_tray`
//! window (never shown; message-only windows get no `TaskbarCreated` broadcast and
//! cannot become foreground for menu dismissal) owned by `instance.rs`: that window's procedure calls [`handle`], and
//! its thread calls [`remove`] on shutdown.
//!
//! Uses the classic (pre-`NOTIFYICON_VERSION_4`) callback semantics: `lParam` of
//! the callback message `WM_APP + 1` is the raw mouse message
//! (`WM_LBUTTONUP`, `WM_RBUTTONUP`, ...).

use crate::hotkey::HotEvent;
use crate::tray::{self, MenuItem};
use crate::chan::Sender;
use std::sync::{Mutex, OnceLock};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, w};

/// Notification callback message.
const WM_TRAYICON: u32 = WM_APP + 1;
/// Posted by [`spawn`]: add the icon (runs on the window thread).
const WM_TRAY_INIT: u32 = WM_APP + 2;
/// Posted when an action failed: show the stored message as a balloon.
const WM_TRAY_BALLOON: u32 = WM_APP + 3;
/// Posted by [`refresh_state`]: the recording state changed (icon, tip).
const WM_TRAY_STATE: u32 = WM_APP + 4;
/// Posted by [`refresh_visible`]: add or remove the icon (`show_tray`).
const WM_TRAY_VISIBLE: u32 = WM_APP + 5;
/// Posted by [`refresh_tip`]: the recorded time changed (the tip only).
const WM_TRAY_TIP: u32 = WM_APP + 6;
/// Classic-callback balloon click (`NIN_BALLOONUSERCLICK`).
const BALLOON_CLICK: u32 = WM_USER + 5;
const ICON_ID: u32 = 1;

static TX: Mutex<Option<Sender<HotEvent>>> = Mutex::new(None);
static BALLOON: Mutex<Option<String>> = Mutex::new(None);
/// The folder the next balloon click opens (set with a "Saved" balloon).
static SAVED: Mutex<Option<std::path::PathBuf>> = Mutex::new(None);
/// The next balloon click opens Settings › Key bindings (a hotkey note).
static KEYS_CLICK: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The balloon just shown is about the hotkeys: its click opens Key bindings.
#[cfg_attr(test, allow(dead_code))]
pub fn click_opens_keys() {
    KEYS_CLICK.store(true, std::sync::atomic::Ordering::SeqCst);
}

fn taskbar_created() -> u32 {
    static MSG: OnceLock<u32> = OnceLock::new();
    *MSG.get_or_init(|| unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) })
}

fn send(ev: HotEvent) {
    if let Some(tx) = TX.lock().unwrap().as_ref() {
        let _ = tx.send(ev);
    }
}

/// Start the tray: remember the event sender and ask the tray window to add the icon.
pub fn spawn(tx: Sender<HotEvent>) {
    *TX.lock().unwrap() = Some(tx);
    unsafe {
        if let Ok(hwnd) = FindWindowW(crate::instance::tray_class(), PCWSTR::null()) {
            let _ = PostMessageW(Some(hwnd), WM_TRAY_INIT, WPARAM(0), LPARAM(0));
        }
    }
}

/// Show `text` as a tray balloon (from any thread; the tray window shows it).
#[cfg_attr(test, allow(dead_code))] // tests collect notes instead (`tray::take_notes`)
pub fn notify(text: &str) {
    *SAVED.lock().unwrap() = None;
    KEYS_CLICK.store(false, std::sync::atomic::Ordering::SeqCst);
    post_balloon(text);
}

fn post_balloon(text: &str) {
    *BALLOON.lock().unwrap() = Some(text.to_string());
    post_tray(WM_TRAY_BALLOON);
}

fn post_tray(msg: u32) {
    unsafe {
        if let Ok(hwnd) = FindWindowW(crate::instance::tray_class(), PCWSTR::null()) {
            let _ = PostMessageW(Some(hwnd), msg, WPARAM(0), LPARAM(0));
        }
    }
}

/// `text` ("Saved <path>…") as a balloon; clicking it opens the folder.
pub fn notify_saved(path: &std::path::Path, text: &str) {
    *SAVED.lock().unwrap() = path.parent().map(|p| p.to_path_buf());
    post_balloon(&format!("{text}
Click to open the folder."));
}

/// `show_tray` changed: the tray window adds or removes the icon.
#[cfg_attr(test, allow(dead_code))]
pub fn refresh_visible() {
    post_tray(WM_TRAY_VISIBLE);
}

/// The recording state changed: swap the icon (red dot) and the tip.
#[cfg_attr(test, allow(dead_code))]
pub fn refresh_state() {
    post_tray(WM_TRAY_STATE);
}

/// The recorded time changed: the tip says it.
#[cfg_attr(test, allow(dead_code))]
pub fn refresh_tip() {
    post_tray(WM_TRAY_TIP);
}

/// Drop the event sender (called when the instance guard goes away).
pub fn clear() {
    *TX.lock().unwrap() = None;
}

fn copy_wide<const N: usize>(dst: &mut [u16; N], s: &str) {
    let mut i = 0;
    for c in s.encode_utf16().take(N - 1) {
        dst[i] = c;
        i += 1;
    }
    dst[i] = 0;
}

fn base_data(hwnd: HWND) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: ICON_ID,
        ..Default::default()
    }
}

/// Cached glyph HICON (as usize; 0 = none yet). Replaced on a state or DPI change.
static GLYPH: Mutex<usize> = Mutex::new(0);

/// Render the app icon at the tray size for `hwnd`'s DPI into a new HICON.
fn build_glyph(hwnd: HWND) -> Option<HICON> {
    use windows::Win32::Graphics::Gdi::*;
    use windows::Win32::UI::HiDpi::{GetDpiForSystem, GetDpiForWindow, GetSystemMetricsForDpi};
    unsafe {
        let mut dpi = GetDpiForWindow(hwnd);
        if dpi == 0 {
            dpi = GetDpiForSystem();
        }
        let size = GetSystemMetricsForDpi(SM_CXSMICON, dpi.max(96)).max(16) as u32;
        let rgba = tray::tray_icon_rgba(size, tray::is_recording());
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: size as i32,
                biHeight: -(size as i32), // top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
        let color = CreateDIBSection(None, &bmi, DIB_RGB_COLORS, &mut bits, None, 0).ok()?;
        if bits.is_null() {
            let _ = DeleteObject(color.into());
            return None;
        }
        let dst = std::slice::from_raw_parts_mut(bits.cast::<u8>(), rgba.len());
        for (d, s) in dst.as_chunks_mut::<4>().0.iter_mut().zip(rgba.as_chunks::<4>().0) {
            *d = [s[2], s[1], s[0], s[3]]; // RGBA -> BGRA, straight alpha
        }
        let mask_bytes = vec![0u8; (size as usize).div_ceil(16) * 2 * size as usize];
        let mask = CreateBitmap(size as i32, size as i32, 1, 1, Some(mask_bytes.as_ptr().cast()));
        let info = ICONINFO { fIcon: true.into(), xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
        let icon = CreateIconIndirect(&info).ok();
        let _ = DeleteObject(color.into());
        let _ = DeleteObject(mask.into());
        icon
    }
}

/// The app icon (resource 1) at the small-icon size: fallback if the glyph fails.
unsafe fn load_icon() -> HICON {
    unsafe {
        let hinst = GetModuleHandleW(PCWSTR::null()).unwrap_or_default();
        let img = LoadImageW(
            Some(hinst.into()),
            PCWSTR(std::ptr::without_provenance(1)),
            IMAGE_ICON,
            GetSystemMetrics(SM_CXSMICON),
            GetSystemMetrics(SM_CYSMICON),
            LR_DEFAULTCOLOR,
        );
        match img {
            Ok(h) if !h.is_invalid() => HICON(h.0),
            _ => LoadIconW(None, IDI_APPLICATION).unwrap_or_default(),
        }
    }
}

/// Cached glyph icon, rendering it on first use.
fn icon(hwnd: HWND) -> HICON {
    let mut g = GLYPH.lock().unwrap();
    if *g == 0 {
        *g = match build_glyph(hwnd) {
            Some(h) => h.0 as usize,
            None => {
                eprintln!("Rustshot: could not draw the tray icon; using the exe's icon");
                return unsafe { load_icon() };
            }
        };
    }
    HICON(*g as *mut _)
}

/// The state or DPI changed: re-render the icon and swap it into the tray icon.
fn refresh_icon(hwnd: HWND) {
    if *GLYPH.lock().unwrap() == 0 {
        return; // not added yet
    }
    let Some(new) = build_glyph(hwnd) else { return };
    let old = std::mem::replace(&mut *GLYPH.lock().unwrap(), new.0 as usize);
    unsafe {
        let mut nid = base_data(hwnd);
        nid.uFlags = NIF_ICON;
        nid.hIcon = new;
        let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
        if old != 0 {
            let _ = DestroyIcon(HICON(old as *mut _));
        }
    }
}

fn tip() -> String {
    tray::tip_text()
}

fn set_tip(hwnd: HWND) {
    unsafe {
        let mut nid = base_data(hwnd);
        nid.uFlags = NIF_TIP;
        copy_wide(&mut nid.szTip, &tip());
        let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
    }
}

fn add_icon(hwnd: HWND) {
    unsafe {
        let mut nid = base_data(hwnd);
        nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
        nid.uCallbackMessage = WM_TRAYICON;
        nid.hIcon = icon(hwnd);
        copy_wide(&mut nid.szTip, &tip());
        if !Shell_NotifyIconW(NIM_ADD, &nid).as_bool() {
            eprintln!("Rustshot: could not add the notification-area icon");
        }
    }
}

/// Remove the icon (called from the window thread before the window is destroyed).
pub fn remove(hwnd: HWND) {
    unsafe {
        let _ = Shell_NotifyIconW(NIM_DELETE, &base_data(hwnd));
    }
}

fn balloon(hwnd: HWND, text: &str) {
    unsafe {
        let mut nid = base_data(hwnd);
        nid.uFlags = NIF_INFO;
        nid.dwInfoFlags = NIIF_INFO;
        copy_wide(&mut nid.szInfoTitle, "Rustshot");
        copy_wide(&mut nid.szInfo, text);
        let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
    }
}

fn show_menu(hwnd: HWND) {
    unsafe {
        let items = tray::current_menu();
        let Ok(menu) = CreatePopupMenu() else { return };
        // Command ids are the flat list's indexes + 1 (submenus included).
        let append = |menu: HMENU, i: usize| {
            let item = items[i];
            if item == MenuItem::Separator {
                let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
                return;
            }
            let mut flags = MF_STRING;
            if item.checked() == Some(true) {
                flags |= MF_CHECKED;
            }
            let label: Vec<u16> = item.label().encode_utf16().chain(Some(0)).collect();
            let _ = AppendMenuW(menu, flags, i + 1, PCWSTR(label.as_ptr()));
        };
        for (i, kids) in tray::nest(&items) {
            if kids.is_empty() {
                append(menu, i);
                continue;
            }
            // "Recent uploads": a submenu (destroyed with its parent).
            let Ok(sub) = CreatePopupMenu() else { continue };
            for k in kids {
                append(sub, k);
            }
            let label: Vec<u16> = items[i].label().encode_utf16().chain(Some(0)).collect();
            let _ = AppendMenuW(menu, MF_POPUP, sub.0 as usize, PCWSTR(label.as_ptr()));
        }
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        // Required so the menu dismisses when the user clicks elsewhere.
        let _ = SetForegroundWindow(hwnd);
        let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON, pt.x, pt.y, Some(0), hwnd, None);
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        if cmd.0 > 0
            && let Some(item) = items.get(cmd.0 as usize - 1)
        {
            run(hwnd, *item);
        }
    }
}

fn run(hwnd: HWND, item: MenuItem) {
    let Some(tx) = TX.lock().unwrap().clone() else { return };
    if let Err(e) = tray::activate(item, &tx) {
        *BALLOON.lock().unwrap() = Some(e);
        unsafe {
            let _ = PostMessageW(Some(hwnd), WM_TRAY_BALLOON, WPARAM(0), LPARAM(0));
        }
    }
}

/// Window-procedure hook; `None` = not a tray message.
pub fn handle(hwnd: HWND, m: u32, _w: WPARAM, l: LPARAM) -> Option<LRESULT> {
    match m {
        WM_TRAY_INIT if tray::is_visible() => add_icon(hwnd),
        WM_TRAY_INIT => {}
        WM_TRAY_VISIBLE if tray::is_visible() => {
            remove(hwnd); // never two
            add_icon(hwnd);
        }
        WM_TRAY_VISIBLE => remove(hwnd),
        WM_TRAY_BALLOON => {
            if let Some(text) = BALLOON.lock().unwrap().take() {
                balloon(hwnd, &text);
            }
        }
        WM_TRAY_STATE => {
            refresh_icon(hwnd);
            set_tip(hwnd);
        }
        WM_TRAY_TIP if tray::is_visible() => set_tip(hwnd),
        WM_TRAY_TIP => {}
        WM_DPICHANGED => refresh_icon(hwnd),
        WM_TRAYICON => match l.0 as u32 {
            // While recording, a click stops it (the menu has the same).
            WM_LBUTTONUP => send(tray::click_event()),
            BALLOON_CLICK => {
                if let Some(dir) = SAVED.lock().unwrap().take() {
                    crate::actions::open_path(&dir);
                } else if KEYS_CLICK.swap(false, std::sync::atomic::Ordering::SeqCst) {
                    crate::settings_ui::show_at(crate::settings_ui::KEYS_GLOBAL);
                }
            }
            WM_RBUTTONUP | WM_CONTEXTMENU => show_menu(hwnd),
            _ => {}
        },
        // Explorer restarted: the notification area was recreated.
        _ if m == taskbar_created() && TX.lock().unwrap().is_some() && tray::is_visible() => add_icon(hwnd),
        _ => return None,
    }
    Some(LRESULT(0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::instance::{Instance, acquire_or_signal};

    fn icon_exists() -> bool {
        unsafe {
            let hwnd = FindWindowW(crate::instance::tray_class(), PCWSTR::null()).unwrap();
            let mut nid = base_data(hwnd);
            nid.uFlags = NIF_TIP;
            copy_wide(&mut nid.szTip, "Rustshot");
            // NIM_MODIFY only succeeds for an icon that exists.
            Shell_NotifyIconW(NIM_MODIFY, &nid).as_bool()
        }
    }

    #[test]
    fn copy_wide_truncates_and_terminates() {
        let mut b = [1u16; 4];
        copy_wide(&mut b, "abcdef");
        assert_eq!(b, [97, 98, 99, 0]);
    }

    #[test]
    #[ignore = "creates the real notification-area icon; fails if a daemon is running"]
    fn live_icon_added_and_removed() {
        let Instance::Primary(g) = acquire_or_signal() else { panic!("a daemon is already running") };
        let (tx, _rx) = crate::chan::channel();
        g.listen(tx.clone());
        crate::tray::spawn(tx, true);
        std::thread::sleep(std::time::Duration::from_millis(1500));
        assert!(icon_exists(), "icon should be present after spawn");
        drop(g);
        // The window is gone after the guard drops, which also removed the icon.
        assert!(unsafe { FindWindowW(crate::instance::tray_class(), PCWSTR::null()) }.is_err());
    }
}
