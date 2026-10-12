//! Where a capture came from, for its History entry: the window that was
//! in front when it started (title and app) and the monitor's name.
//! Windows asks the system; elsewhere both are empty for now (History then
//! names a capture by its file).

/// A friendlier name for a program's file name ("chrome" is "Chrome",
/// "Code" is "VS Code"); others get a capital first letter.
#[cfg_attr(not(any(windows, test)), allow(dead_code))]
pub fn app_name(exe_stem: &str) -> String {
    let s = exe_stem.trim();
    let known: &[(&str, &str)] = &[
        ("chrome", "Chrome"),
        ("msedge", "Edge"),
        ("firefox", "Firefox"),
        ("code", "VS Code"),
        ("windowsterminal", "Terminal"),
        ("explorer", "Explorer"),
        ("winword", "Word"),
        ("excel", "Excel"),
        ("powerpnt", "PowerPoint"),
        ("ms-teams", "Teams"),
        ("teams", "Teams"),
        ("devenv", "Visual Studio"),
        ("idea64", "IntelliJ IDEA"),
        ("acrord32", "Acrobat Reader"),
    ];
    if let Some((_, n)) = known.iter().find(|(k, _)| s.eq_ignore_ascii_case(k)) {
        return n.to_string();
    }
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

/// The window in front now: (title, app). Empty where unknown.
#[cfg(windows)]
pub fn foreground() -> (String, String) {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW};
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowTextW, GetWindowThreadProcessId};
    use windows::core::PWSTR;
    unsafe {
        let h = GetForegroundWindow();
        if h.is_invalid() {
            return Default::default();
        }
        let mut buf = [0u16; 512];
        let n = GetWindowTextW(h, &mut buf).max(0) as usize;
        let title = String::from_utf16_lossy(&buf[..n]);
        let mut pid = 0u32;
        GetWindowThreadProcessId(h, Some(&mut pid));
        if pid == std::process::id() {
            // Our own window (History, Settings): no source.
            return Default::default();
        }
        let mut app = String::new();
        if let Ok(p) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
            let mut b = [0u16; 1024];
            let mut len = b.len() as u32;
            if QueryFullProcessImageNameW(p, PROCESS_NAME_WIN32, PWSTR(b.as_mut_ptr()), &mut len).is_ok() {
                let path = std::path::PathBuf::from(String::from_utf16_lossy(&b[..len as usize]));
                app = path.file_stem().map(|s| app_name(&s.to_string_lossy())).unwrap_or_default();
            }
            let _ = CloseHandle(p);
        }
        (title, app)
    }
}

#[cfg(not(windows))]
pub fn foreground() -> (String, String) {
    Default::default()
}

/// The name of the monitor at global point (`x`, `y`) ("DELL U2720Q"),
/// empty when unknown.
#[cfg(windows)]
pub fn monitor_name(x: i32, y: i32) -> String {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::Graphics::Gdi::{DISPLAY_DEVICEW, EnumDisplayDevicesW, GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MONITORINFOEXW, MonitorFromPoint};
    use windows::core::PCWSTR;
    unsafe {
        let m = MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        if !GetMonitorInfoW(m, &mut info as *mut MONITORINFOEXW as *mut MONITORINFO).as_bool() {
            return String::new();
        }
        let mut dd = DISPLAY_DEVICEW { cb: std::mem::size_of::<DISPLAY_DEVICEW>() as u32, ..Default::default() };
        if !EnumDisplayDevicesW(PCWSTR(info.szDevice.as_ptr()), 0, &mut dd, 0).as_bool() {
            return String::new();
        }
        let n = dd.DeviceString.iter().position(|&c| c == 0).unwrap_or(dd.DeviceString.len());
        let name = String::from_utf16_lossy(&dd.DeviceString[..n]);
        // "Generic PnP Monitor" says nothing.
        if name.contains("Generic") { "Display".into() } else { name }
    }
}

#[cfg(not(windows))]
pub fn monitor_name(_x: i32, _y: i32) -> String {
    String::new()
}

/// "Built-in Display (2×)", "DELL U2720Q (1.5×)", "DELL U2720Q".
pub fn monitor_text(name: &str, scale: f32) -> String {
    let n = if name.trim().is_empty() { "Display" } else { name.trim() };
    if (scale - 1.0).abs() < 0.01 || !scale.is_finite() || scale <= 0.0 {
        n.to_string()
    } else {
        let s = format!("{scale:.2}");
        let s = s.trim_end_matches('0').trim_end_matches('.');
        format!("{n} ({s}\u{d7})")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_names() {
        assert_eq!(app_name("chrome"), "Chrome");
        assert_eq!(app_name("Code"), "VS Code");
        assert_eq!(app_name("slack"), "Slack");
        assert_eq!(app_name("Figma"), "Figma");
        assert_eq!(app_name(""), "");
        assert_eq!(monitor_text("Built-in Display", 2.0), "Built-in Display (2\u{d7})");
        assert_eq!(monitor_text("DELL", 1.5), "DELL (1.5\u{d7})");
        assert_eq!(monitor_text("", 1.0), "Display");
        assert_eq!(monitor_text("X", 1.25), "X (1.25\u{d7})");
    }
}
