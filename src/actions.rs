//! Tray actions shared by every backend.
#![cfg_attr(not(windows), allow(dead_code))] // used by the macOS/Linux trays (later tasks)

use crate::update_ui::DialogState;
use std::path::Path;

pub fn open_config() {
    match crate::config::ensure_config_file() {
        Ok(p) => open_path(&p),
        Err(e) => eprintln!("cannot create the config file: {e}"),
    }
}

/// Open a local file with the OS default handler (never a URL).
pub fn open_path(path: &Path) {
    #[cfg(not(windows))]
    use std::process::Command;
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows::Win32::UI::Shell::ShellExecuteW;
        use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
        use windows::core::{PCWSTR, w};
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let r = unsafe {
            ShellExecuteW(None, w!("open"), PCWSTR(wide.as_ptr()), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL)
        };
        if r.0 as isize <= 32 {
            // No association (e.g. for .toml): fall back to Notepad.
            if let Some(exe) = crate::proc_win::system_exe("notepad.exe") {
                let _ = crate::proc_win::spawn_detached(&exe, &[path.as_os_str()], &[]);
            }
        }
    }
    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut c = Command::new("open");
        c.arg(path);
        c
    };
    #[cfg(not(any(windows, target_os = "macos")))]
    let mut cmd = {
        let mut c = Command::new("xdg-open");
        c.arg(path);
        c
    };
    #[cfg(not(windows))]
    let _ = cmd.spawn();
}

/// Tray "Check for updates": the update dialog opens at once and shows
/// the result of the check (store installs: who updates them).
pub fn check_updates() {
    match crate::update::update_channel() {
        Some(store) => crate::update_ui::show(DialogState::Managed(store)),
        None => crate::update_ui::check(),
    }
}

/// Flip "Start at login"; returns the new state.
pub fn toggle_autostart() -> Result<bool, String> {
    let on = !crate::autostart::is_enabled();
    crate::autostart::set(on)?;
    Ok(on)
}

/// PowerNapps' short link: a 302 on powernapps.net that adds the UTM
/// parameters, so they can change without an app update.
pub const POWERNAPPS: &str = "https://powernapps.net/r/rustshot";

/// Imgur's page for registering an app (a client ID of one's own):
/// Settings › Upload's "Get a key".
pub const IMGUR_ADD_CLIENT: &str = "https://api.imgur.com/oauth2/addclient";

/// The links of Settings → About (the first five), PowerNapps' and
/// Settings › Upload's "Get a key": the only URLs [`open_link`] opens.
pub const LINKS: [(crate::i18n::Key, &str); 7] = [
    (key!("settings.about.link.website"), POWERNAPPS),
    (key!("settings.about.link.source"), "https://github.com/nappsllc/rustshot"),
    (key!("settings.about.link.issue"), "https://github.com/nappsllc/rustshot/issues"),
    (key!("settings.about.link.whats_new"), "https://github.com/nappsllc/rustshot/releases"),
    (key!("settings.about.link.licenses"), "https://github.com/nappsllc/rustshot/blob/main/THIRD_PARTY_NOTICES.md"),
    (key!("settings.about.link.powernapps"), POWERNAPPS),
    (key!("settings.upload.get_key"), IMGUR_ADD_CLIENT),
];

/// Open one of [`LINKS`] in the default browser; any other URL is refused.
pub fn open_link(url: &str) {
    if !LINKS.iter().any(|(_, u)| *u == url) {
        eprintln!("refusing to open an unexpected URL");
        return;
    }
    #[cfg(windows)]
    {
        use std::ffi::OsStr;
        if let Some(exe) = crate::proc_win::system_exe("rundll32.exe") {
            let _ = crate::proc_win::spawn_detached(&exe, &[OsStr::new("url.dll,FileProtocolHandler"), OsStr::new(url)], &[]);
        }
    }
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(url).spawn();
    #[cfg(not(any(windows, target_os = "macos")))]
    let _ = std::process::Command::new("xdg-open").arg(url).spawn();
}

/// The tasks of a capture with `copy_after_capture` applied: one that
/// saves or uploads also copies the image (once).
pub fn after_capture(mut tasks: Vec<crate::export::Task>, cfg: &crate::config::Config) -> Vec<crate::export::Task> {
    use crate::export::Task;
    let keeps = tasks.iter().any(|t| matches!(t, Task::Save { .. } | Task::SaveDesktop | Task::Upload));
    if cfg.copy_after_capture && keeps && !tasks.contains(&Task::Copy) {
        tasks.push(Task::Copy);
    }
    tasks
}

/// The shutter sound (`shutter_sound`).
pub fn shutter() {
    crate::sound::shutter();
}

/// The monitor to capture for Repeat last capture: `None` (the overlay's
/// usual span) when every monitor shares one scale, else the one holding
/// the centre of `r` (global physical px).
pub fn monitor_for_region(mons: &[crate::capture::MonInfo], r: (i32, i32, u32, u32)) -> Option<u32> {
    if crate::capture::uniform_scale(mons).is_some() {
        return None;
    }
    let (cx, cy) = (r.0 + r.2 as i32 / 2, r.1 + r.3 as i32 / 2);
    mons.iter()
        .position(|m| cx >= m.x && cy >= m.y && cx < m.x + m.w as i32 && cy < m.y + m.h as i32)
        .map(|i| i as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::export::Task;

    #[test]
    fn copy_after_capture_adds_one_copy_to_saves_and_uploads() {
        let on = Config { copy_after_capture: true, ..Config::default() };
        let save = || vec![Task::Save { path: None, ask: false }];
        assert_eq!(after_capture(save(), &Config::default()), save(), "off: unchanged");
        assert_eq!(after_capture(save(), &on), vec![Task::Save { path: None, ask: false }, Task::Copy]);
        assert_eq!(after_capture(vec![Task::SaveDesktop], &on), vec![Task::SaveDesktop, Task::Copy]);
        assert_eq!(after_capture(vec![Task::Upload], &on), vec![Task::Upload, Task::Copy]);
        assert_eq!(after_capture(vec![Task::Copy], &on), vec![Task::Copy], "a copy is not doubled");
        assert_eq!(after_capture(vec![Task::Copy, Task::Upload], &on), vec![Task::Copy, Task::Upload]);
        assert_eq!(after_capture(vec![], &on), vec![], "cancelled / recording: nothing");
    }

    #[test]
    fn repeat_monitor_only_when_scales_differ() {
        use crate::capture::MonInfo;
        let m = |x, scale| MonInfo { x, y: 0, w: 1920, h: 1080, scale, primary: x == 0 };
        let same = [m(0, 1.0), m(1920, 1.0)];
        assert_eq!(monitor_for_region(&same, (2000, 100, 300, 200)), None);
        let mixed = [m(0, 1.0), m(1920, 1.5)];
        assert_eq!(monitor_for_region(&mixed, (2000, 100, 300, 200)), Some(1));
        assert_eq!(monitor_for_region(&mixed, (10, 10, 30, 20)), Some(0));
        assert_eq!(monitor_for_region(&mixed, (-900, 10, 30, 20)), None, "off every monitor");
    }
}
