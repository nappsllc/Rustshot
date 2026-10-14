//! Win+Shift+S and Print Screen open Rustshot (Windows): both launch the
//! `ms-screenclip:` URI, and Rustshot can be offered as its handler.
//!
//! Registration is per user (HKCU, no administrator rights), as the
//! Default Programs model does it: a ProgID whose `shell\open\command`
//! runs `"<exe>" screenclip "%1"`, a `Capabilities` key claiming
//! `ms-screenclip` for it and a `RegisteredApplications` entry pointing
//! there. Windows keeps the user's choice of handler in a hash-protected
//! `UserChoice` key that no app may write, so registering only makes
//! Rustshot a candidate: the next Win+Shift+S asks which app to use (or
//! the user picks it under Settings > Apps > Default apps > "Choose
//! defaults by link type" > MS-SCREENCLIP). Writing
//! `HKCU\Software\Classes\ms-screenclip\shell\open\command` directly
//! would be ignored while that choice exists (the Snipping Tool's AppX
//! ProgID), and it would overwrite a key that is not ours.
//!
//! Nothing that exists before is overwritten except our own keys; the
//! `RegisteredApplications` value, the one shared name, is backed up
//! when it held something else and restored on unregister. Unregistering
//! also removes the user's choice when it points at our ProgID, so
//! Windows goes back to its default handler (the Snipping Tool).
//!
//! Every name is in [`Names`], so tests register a throwaway scheme.
//! The URI parsing ([`parse_uri`]) is pure and tested everywhere.
#![cfg_attr(not(windows), allow(dead_code))]

/// What a capture request asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Rectangle (and freeform, which Rustshot draws as a rectangle).
    Region,
    /// Pick a window.
    Window,
    /// The whole screen, saved at once (the full-screen hotkey).
    Full,
    /// Area selection in record mode.
    Record,
}

impl Mode {
    /// The daemon's hotkey slot for this mode (see `hotkey::slot_event`).
    pub fn slot(self) -> usize {
        match self {
            Mode::Region => 0,
            Mode::Record => 2,
            Mode::Full => 3,
            Mode::Window => 4,
        }
    }
}

/// A parsed `ms-screenclip:` URI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request {
    /// `None`: nothing to capture (`ms-screenclip://discover`, which an
    /// unpackaged handler cannot answer).
    pub mode: Option<Mode>,
    /// Seconds to wait before capturing (`delayInSeconds`, 0..=30).
    pub delay_s: u32,
}

/// The longest delay a URI may ask for (the Snipping Tool's limit).
pub const MAX_DELAY_S: u32 = 30;

/// Read a URI as Windows passes it: the legacy form
/// (`ms-screenclip:?source=HotKey&clippingMode=Window&delayInSeconds=3`,
/// `type=recording`) and the current one
/// (`ms-screenclip://capture/image?window`, `capture/video`,
/// `discover`). Names and values are case-insensitive; anything unknown
/// is ignored and a request that names no mode is a region capture. The
/// URI comes from any caller, so nothing but the mode and a bounded
/// delay is taken from it.
pub fn parse_uri(uri: &str) -> Request {
    let uri = uri.trim().trim_matches('"').trim();
    let rest = uri.split_once(':').map_or("", |(_, r)| r);
    let (target, query) = rest.split_once('?').unwrap_or((rest, ""));
    let target = target.trim_matches('/').to_ascii_lowercase();
    let mut mode = None;
    let mut delay_s = 0;
    match target.split_once('/').unwrap_or((&target, "")) {
        ("discover", _) => return Request { mode: None, delay_s: 0 },
        ("capture", "video") => mode = Some(Mode::Record),
        _ => {}
    }
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        let (k, v) = (k.trim().to_ascii_lowercase(), v.trim().to_ascii_lowercase());
        let named = match (k.as_str(), v.as_str()) {
            ("clippingmode", m) | (m, "") => match m {
                "rectangle" | "freeform" => Some(Mode::Region),
                "window" => Some(Mode::Window),
                "fullscreen" | "full" => Some(Mode::Full),
                _ => None,
            },
            ("type", "recording" | "video") => Some(Mode::Record),
            ("delayinseconds", n) => {
                if let Ok(n) = n.parse::<u32>() {
                    delay_s = n.min(MAX_DELAY_S);
                }
                None
            }
            _ => None,
        };
        // A recording stays one whatever shape it names.
        if mode != Some(Mode::Record) && named.is_some() {
            mode = named;
        }
    }
    Request { mode: Some(mode.unwrap_or(Mode::Region)), delay_s }
}

/// A URI Windows launches for us, given as the first argument instead of
/// after `screenclip` (`rustshot ms-screenclip:?...`).
pub fn is_screenclip_uri(arg: &str) -> bool {
    arg.trim_matches('"').get(..14).is_some_and(|p| p.eq_ignore_ascii_case("ms-screenclip:"))
}

/// The handler's command line: `"<exe>" screenclip "%1"`.
pub fn open_command(exe: &str) -> String {
    format!("\"{exe}\" screenclip \"%1\"")
}

/// The executable of a command line made by [`open_command`] (or any
/// command starting with a quoted or bare path).
pub fn command_exe(cmd: &str) -> Option<&str> {
    let cmd = cmd.trim_start();
    let exe = match cmd.strip_prefix('"') {
        Some(r) => r.split('"').next()?,
        None => cmd.split(' ').next()?,
    };
    (!exe.is_empty()).then_some(exe)
}

/// Every registry name the registration uses (all under HKCU).
#[derive(Clone, Debug)]
pub struct Names {
    /// The URI scheme claimed (`ms-screenclip`).
    pub scheme: String,
    /// Our ProgID, under `Software\Classes`.
    pub progid: String,
    /// The `RegisteredApplications` value name and `ApplicationName`.
    pub app: String,
    /// Our key; `Capabilities` and the backup live under it.
    pub app_key: String,
}

impl Names {
    /// The real registration.
    pub fn real() -> Names {
        Names {
            scheme: "ms-screenclip".into(),
            progid: "Rustshot.ScreenClip".into(),
            app: "Rustshot".into(),
            app_key: r"Software\Rustshot".into(),
        }
    }

    pub fn progid_key(&self) -> String {
        format!(r"Software\Classes\{}", self.progid)
    }

    pub fn command_key(&self) -> String {
        format!(r"{}\shell\open\command", self.progid_key())
    }

    pub fn icon_key(&self) -> String {
        format!(r"{}\DefaultIcon", self.progid_key())
    }

    /// `RegisteredApplications` points here.
    pub fn capabilities(&self) -> String {
        format!(r"{}\Capabilities", self.app_key)
    }

    pub fn url_associations(&self) -> String {
        format!(r"{}\URLAssociations", self.capabilities())
    }

    /// The previous `RegisteredApplications` value, when it was not ours.
    pub fn backup_key(&self) -> String {
        format!(r"{}\ScreenclipBackup", self.app_key)
    }

    /// Where Windows keeps the user's choice of handler.
    pub fn user_choice_root(&self) -> String {
        format!(r"Software\Microsoft\Windows\Shell\Associations\UrlAssociations\{}", self.scheme)
    }
}

pub const REGISTERED_APPS: &str = r"Software\RegisteredApplications";

// --- Who opens Win+Shift+S --------------------------------------------------

/// A registry hive the handler check reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Root {
    /// `HKEY_CURRENT_USER`.
    User,
    /// `HKEY_CLASSES_ROOT` (the user's and the machine's classes merged).
    Classes,
}

/// The registry reads the handler check makes (tests fake them).
pub trait RegRead {
    /// String value `name` (`None`: the default value) of `key`.
    fn get(&self, root: Root, key: &str, name: Option<&str>) -> Option<String>;
    /// `key` exists.
    fn exists(&self, root: Root, key: &str) -> bool;
    /// An indirect string (`@{Package?ms-resource://…}`, `@file.dll,-12`)
    /// as text, when it resolves.
    fn indirect(&self, s: &str) -> Option<String> {
        let _ = s;
        None
    }
    /// The program a command names is there.
    fn file_exists(&self, path: &str) -> bool {
        std::path::Path::new(path).is_file()
    }
}

/// Which app Windows opens for the scheme (Win+Shift+S).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Handler {
    /// Rustshot.
    Ours,
    /// Another app that is installed (the Snipping Tool, usually).
    Other { progid: String, name: String },
    /// The user's choice names a ProgID that no longer opens anything (an
    /// uninstalled Snipping Tool): Windows shows "This file does not have
    /// an app associated with it" and never asks.
    Broken { progid: String },
    /// No choice: Windows asks (when there is a candidate) or uses its
    /// default.
    None,
}

/// The ProgID the user chose for the scheme: `UserChoiceLatest`'s (newer
/// Windows 11) when there is one, else `UserChoice`'s.
pub fn user_choice_in(r: &impl RegRead, n: &Names) -> Option<String> {
    let root = n.user_choice_root();
    let pick = |key: String| r.get(Root::User, &key, Some("ProgId")).map(|p| p.trim().to_string()).filter(|p| !p.is_empty());
    pick(format!(r"{root}\UserChoiceLatest\ProgId")).or_else(|| pick(format!(r"{root}\UserChoice")))
}

/// A packaged (Store) app's ProgID: `AppX` and a hash.
fn is_appx(progid: &str) -> bool {
    progid.get(..4).is_some_and(|p| p.eq_ignore_ascii_case("appx"))
}

/// `progid` opens something: it has a `shell\open\command` (in the user's
/// classes or the merged ones) whose program, when the command names one
/// by its full path, is still there; or it is a packaged app's ProgID that
/// is still registered (an installed AppX package keeps its `AppX…` key
/// under `HKCR`; uninstalling the package removes it).
pub fn resolves(r: &impl RegRead, progid: &str) -> bool {
    if progid.is_empty() || progid.contains(['\\', '/']) {
        return false;
    }
    if is_appx(progid) && r.exists(Root::Classes, progid) {
        return true;
    }
    let user = format!(r"Software\Classes\{progid}\shell\open\command");
    let merged = format!(r"{progid}\shell\open\command");
    let cmd = if r.exists(Root::User, &user) {
        r.get(Root::User, &user, None)
    } else if r.exists(Root::Classes, &merged) {
        r.get(Root::Classes, &merged, None)
    } else {
        return false;
    };
    // A command naming a program that is gone (an uninstalled app's stale
    // key) opens nothing either. Only a plain absolute path is checked: no
    // command (DelegateExecute), `%vars%` or bare names are trusted.
    match cmd.as_deref().and_then(command_exe) {
        Some(exe) if is_absolute_windows_path(exe) && !exe.contains('%') => r.file_exists(exe),
        _ => true,
    }
}

/// `C:\…` or `\\server\…`.
fn is_absolute_windows_path(p: &str) -> bool {
    let b = p.as_bytes();
    (b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/')) || p.starts_with(r"\\")
}

/// A readable name for the app behind `progid`: a packaged app's
/// `Application\ApplicationName` (resolved when indirect), else the
/// ProgID's description, else its command's program name, else the
/// ProgID itself.
pub fn app_name(r: &impl RegRead, progid: &str) -> String {
    let text = |s: Option<String>| -> Option<String> {
        let s = s?.trim().to_string();
        let s = if s.starts_with('@') { r.indirect(&s)? } else { s };
        let s = s.trim().to_string();
        (!s.is_empty() && !s.starts_with('@')).then_some(s)
    };
    let classes = |key: &str, name: Option<&str>| {
        r.get(Root::User, &format!(r"Software\Classes\{key}"), name).or_else(|| r.get(Root::Classes, key, name))
    };
    if let Some(s) = text(classes(&format!(r"{progid}\Application"), Some("ApplicationName"))) {
        return s;
    }
    if let Some(s) = text(classes(progid, None)).filter(|s| !s.to_ascii_lowercase().starts_with("url:")) {
        return s;
    }
    let cmd = classes(&format!(r"{progid}\shell\open\command"), None);
    // A Windows path, split by hand (tests run everywhere).
    let stem = cmd.as_deref().and_then(command_exe).map(|e| {
        let file = e.rsplit(['\\', '/']).next().unwrap_or(e);
        file.rsplit_once('.').map_or(file, |(s, _)| s).to_string()
    });
    if let Some(stem) = stem.filter(|s| !s.is_empty()) {
        return stem;
    }
    progid.to_string()
}

/// Which app the scheme opens, from the registry `r`.
pub fn handler_in(r: &impl RegRead, n: &Names) -> Handler {
    let Some(progid) = user_choice_in(r, n) else { return Handler::None };
    if !resolves(r, &progid) {
        return Handler::Broken { progid };
    }
    if progid.eq_ignore_ascii_case(&n.progid) {
        return Handler::Ours;
    }
    let name = app_name(r, &progid);
    Handler::Other { progid, name }
}

/// The message that says how to pick Rustshot (Windows lets no app do it).
pub const GUIDE_TITLE: crate::i18n::Key = key!("screenclip.guide.title");
pub const GUIDE_STEPS: crate::i18n::Key = key!("screenclip.guide.steps");
/// Said first when the choice names an app that is gone.
pub const GUIDE_BROKEN: crate::i18n::Key = key!("screenclip.guide.broken");
pub const GUIDE_OPEN: crate::i18n::Key = key!("screenclip.guide.open");
pub const GUIDE_LATER: crate::i18n::Key = key!("screenclip.guide.later");
pub const GUIDE_DONT_SHOW: crate::i18n::Key = key!("screenclip.guide.dont_show");
/// The toast once Windows uses Rustshot.
pub const NOW_OURS: crate::i18n::Key = key!("screenclip.now_ours");

/// The guide's body for a choice that is `broken` or not.
pub fn guide_text(broken: bool) -> String {
    if broken { format!("{GUIDE_BROKEN}\n{GUIDE_STEPS}") } else { GUIDE_STEPS.to_string() }
}

/// How a handler line reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Good,
    Plain,
    Bad,
}

/// The Settings row's line when the tray icon is off and Windows does not
/// open Rustshot: no shortcut reaches Rustshot then.
pub const NO_SHORTCUT: crate::i18n::Key = key!("screenclip.no_shortcut");

/// The Settings row's line for `h`; `show_tray`: the tray icon is on
/// (off, anything but Rustshot is [`NO_SHORTCUT`], a link to the guide).
pub fn state_line(h: &Handler, show_tray: bool) -> (String, Tone) {
    if !show_tray && *h != Handler::Ours {
        return (NO_SHORTCUT.to_string(), Tone::Bad);
    }
    match h {
        Handler::Ours => (tr!("screenclip.state.ours").into(), Tone::Good),
        Handler::Other { name, .. } => (tr!("screenclip.state.other", name), Tone::Plain),
        Handler::Broken { .. } => (tr!("screenclip.state.broken").into(), Tone::Bad),
        Handler::None => (tr!("screenclip.state.none").into(), Tone::Plain),
    }
}

/// The Settings page the guide's button opens: the plain Default apps
/// page, where "Choose defaults by link type" is (the guide's steps).
pub const DEFAULT_APPS_URI: &str = "ms-settings:defaultapps";

/// Open Windows' Default apps; the error when Windows did not.
pub fn open_default_apps() -> Result<(), String> {
    #[cfg(windows)]
    {
        reg::shell_open(DEFAULT_APPS_URI)
    }
    #[cfg(not(windows))]
    Err("only on Windows".into())
}

/// The guide's line when Default apps did not open.
pub fn open_failed_text(e: &str) -> String {
    tr!("screenclip.open_failed", e = e.trim().trim_end_matches('.'))
}

/// The registry: writes only our own HKCU keys; reads HKCU and (for the
/// handler check, read-only) HKCR.
#[cfg(windows)]
pub mod reg {
    use super::*;
    use std::path::Path;
    use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, WIN32_ERROR};
    use windows::Win32::System::Registry::*;
    use windows::core::{HSTRING, PCWSTR};

    fn err(what: &str, key: &str, r: WIN32_ERROR) -> String {
        format!("{what} {key} failed: {}", windows::core::Error::from(r.to_hresult()).message())
    }

    /// `name` (`None`: the default value) of `key`, if it is a string.
    pub fn get(key: &str, name: Option<&str>) -> Option<String> {
        get_in(HKEY_CURRENT_USER, key, name, RRF_RT_REG_SZ)
    }

    fn get_in(root: HKEY, key: &str, name: Option<&str>, flags: REG_ROUTINE_FLAGS) -> Option<String> {
        let key = HSTRING::from(key);
        let name = name.map(HSTRING::from);
        let pname = name.as_ref().map_or(PCWSTR::null(), |n| PCWSTR(n.as_ptr()));
        unsafe {
            let mut len = 0u32;
            let r = RegGetValueW(root, &key, pname, flags, None, None, Some(&mut len));
            if r != ERROR_SUCCESS {
                return None;
            }
            let mut buf = vec![0u16; (len as usize).div_ceil(2) + 1];
            let mut len = (buf.len() * 2) as u32;
            let r = RegGetValueW(root, &key, pname, flags, None, Some(buf.as_mut_ptr().cast()), Some(&mut len));
            if r != ERROR_SUCCESS {
                return None;
            }
            let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
            Some(String::from_utf16_lossy(&buf[..n]))
        }
    }

    fn exists_in(root: HKEY, key: &str) -> bool {
        let mut h = HKEY::default();
        let r = unsafe { RegOpenKeyExW(root, &HSTRING::from(key), None, KEY_READ, &mut h) };
        if r == ERROR_SUCCESS {
            unsafe {
                let _ = RegCloseKey(h);
            }
            true
        } else {
            false
        }
    }

    /// The live registry, read only.
    pub struct Live;

    impl RegRead for Live {
        fn get(&self, root: Root, key: &str, name: Option<&str>) -> Option<String> {
            let h = match root {
                Root::User => HKEY_CURRENT_USER,
                Root::Classes => HKEY_CLASSES_ROOT,
            };
            get_in(h, key, name, RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ | RRF_NOEXPAND)
        }

        fn exists(&self, root: Root, key: &str) -> bool {
            exists_in(if root == Root::User { HKEY_CURRENT_USER } else { HKEY_CLASSES_ROOT }, key)
        }

        fn indirect(&self, s: &str) -> Option<String> {
            let mut buf = vec![0u16; 512];
            unsafe { windows::Win32::UI::Shell::SHLoadIndirectString(&HSTRING::from(s), &mut buf, None) }.ok()?;
            let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
            Some(String::from_utf16_lossy(&buf[..n]))
        }
    }

    /// Open a `ms-settings:` page.
    /// Open a `ms-settings:` page (COM initialized on this thread, as
    /// `ShellExecute` wants for URI handlers); the error when it did not.
    pub fn shell_open(uri: &str) -> Result<(), String> {
        use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoUninitialize};
        use windows::Win32::UI::Shell::ShellExecuteW;
        use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
        if !uri.starts_with("ms-settings:") {
            return Err(format!("not a Settings page: {uri}"));
        }
        let com = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };
        let h = unsafe { ShellExecuteW(None, &HSTRING::from("open"), &HSTRING::from(uri), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL) };
        // Above 32 is success; the rest are error codes.
        let res = if h.0 as isize > 32 { Ok(()) } else { Err(windows::core::Error::from_thread().message()) };
        if com.is_ok() {
            unsafe { CoUninitialize() };
        }
        res
    }

    /// Write a string value (creating the key).
    pub fn set(key: &str, name: Option<&str>, value: &str) -> Result<(), String> {
        let data: Vec<u16> = value.encode_utf16().chain(Some(0)).collect();
        let hkey = HSTRING::from(key);
        let name = name.map(HSTRING::from);
        let pname = name.as_ref().map_or(PCWSTR::null(), |n| PCWSTR(n.as_ptr()));
        let r = unsafe {
            RegSetKeyValueW(HKEY_CURRENT_USER, &hkey, pname, REG_SZ.0, Some(data.as_ptr().cast()), (data.len() * 2) as u32)
        };
        if r != ERROR_SUCCESS {
            return Err(err("writing", key, r));
        }
        Ok(())
    }

    /// Remove a value; one that is not there is fine.
    pub fn delete_value(key: &str, name: &str) -> Result<(), String> {
        let r = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, &HSTRING::from(key), &HSTRING::from(name)) };
        if r != ERROR_SUCCESS && r != ERROR_FILE_NOT_FOUND {
            return Err(err("deleting a value of", key, r));
        }
        Ok(())
    }

    /// Remove a key and everything under it; one that is not there is fine.
    pub fn delete_tree(key: &str) -> Result<(), String> {
        let r = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, &HSTRING::from(key)) };
        if r != ERROR_SUCCESS && r != ERROR_FILE_NOT_FOUND {
            return Err(err("deleting", key, r));
        }
        Ok(())
    }

    #[cfg(test)]
    pub fn key_exists(key: &str) -> bool {
        exists_in(HKEY_CURRENT_USER, key)
    }

    /// Our registration is complete: the command, the capability and its
    /// `RegisteredApplications` entry.
    pub fn is_registered(n: &Names) -> bool {
        get(&n.command_key(), None).is_some_and(|c| c.contains(" screenclip "))
            && get(&n.url_associations(), Some(&n.scheme)).is_some_and(|p| p.eq_ignore_ascii_case(&n.progid))
            && get(REGISTERED_APPS, Some(&n.app)).is_some_and(|p| p.eq_ignore_ascii_case(&n.capabilities()))
    }

    /// The executable our command runs, while registered.
    pub fn registered_exe(n: &Names) -> Option<String> {
        get(&n.command_key(), None).and_then(|c| command_exe(&c).map(str::to_owned))
    }

    /// Which app the scheme opens now.
    pub fn handler(n: &Names) -> Handler {
        handler_in(&Live, n)
    }

    /// The user chose our ProgID ([`user_choice_in`]).
    #[cfg(test)]
    pub fn is_default(n: &Names) -> bool {
        user_choice_in(&Live, n).is_some_and(|p| p.eq_ignore_ascii_case(&n.progid))
    }

    /// Offer `exe` as the scheme's handler.
    pub fn register(n: &Names, exe: &Path) -> Result<(), String> {
        let exe = exe.display().to_string();
        let pk = n.progid_key();
        set(&pk, None, "Rustshot screen capture")?;
        set(&pk, Some("URL Protocol"), "")?;
        set(&n.icon_key(), None, &format!("\"{exe}\",0"))?;
        set(&n.command_key(), None, &open_command(&exe))?;
        let cap = n.capabilities();
        set(&cap, Some("ApplicationName"), &n.app)?;
        set(&cap, Some("ApplicationDescription"), "Capture and annotate a screenshot")?;
        set(&n.url_associations(), Some(&n.scheme), &n.progid)?;
        // The one shared name: keep what was there (once).
        if let Some(old) = get(REGISTERED_APPS, Some(&n.app))
            && !old.eq_ignore_ascii_case(&cap)
            && get(&n.backup_key(), Some("RegisteredApplications")).is_none()
        {
            set(&n.backup_key(), Some("RegisteredApplications"), &old)?;
        }
        set(REGISTERED_APPS, Some(&n.app), &cap)
    }

    /// Undo [`register`]: our keys go, a backed-up `RegisteredApplications`
    /// value comes back, and a user choice of our ProgID is removed so
    /// Windows uses its default handler again.
    pub fn unregister(n: &Names) -> Result<(), String> {
        let root = n.user_choice_root();
        let latest = format!(r"{root}\UserChoiceLatest");
        if get(&format!(r"{latest}\ProgId"), Some("ProgId")).is_some_and(|p| p.eq_ignore_ascii_case(&n.progid)) {
            delete_tree(&latest)?;
        }
        let choice = format!(r"{root}\UserChoice");
        if get(&choice, Some("ProgId")).is_some_and(|p| p.eq_ignore_ascii_case(&n.progid)) {
            delete_tree(&choice)?;
        }
        match get(&n.backup_key(), Some("RegisteredApplications")) {
            Some(old) => set(REGISTERED_APPS, Some(&n.app), &old)?,
            None => {
                if get(REGISTERED_APPS, Some(&n.app)).is_some_and(|p| p.eq_ignore_ascii_case(&n.capabilities())) {
                    delete_value(REGISTERED_APPS, &n.app)?;
                }
            }
        }
        delete_tree(&n.backup_key())?;
        delete_tree(&n.capabilities())?;
        delete_tree(&n.progid_key())
    }

    /// Point a registration at `exe` when the executable it runs is gone
    /// (a portable exe that moved); true when it was rewritten. A
    /// registration of another exe that still exists (another copy of
    /// Rustshot, a dev build) is left alone.
    pub fn repoint(n: &Names, exe: &Path) -> Result<bool, String> {
        if !is_registered(n) {
            return Ok(false);
        }
        let Some(old) = registered_exe(n) else { return Ok(false) };
        if Path::new(&old) == exe || Path::new(&old).exists() {
            return Ok(false);
        }
        let exe = exe.display().to_string();
        set(&n.icon_key(), None, &format!("\"{exe}\",0"))?;
        set(&n.command_key(), None, &open_command(&exe))?;
        Ok(true)
    }

    /// Tell the shell that handlers changed (so the next launch offers us).
    pub fn notify_shell() {
        use windows::Win32::UI::Shell::{SHCNE_ASSOCCHANGED, SHCNF_IDLIST, SHChangeNotify};
        unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None) };
    }
}

/// Registering is possible here: Windows, not an MSIX package (a packaged
/// app's HKCU writes are virtualised, so the shell would never see them).
pub fn available() -> bool {
    cfg!(windows) && crate::update::managed_install().is_none()
}

/// The toggle's state, from the registry: `None` where it is not offered.
pub fn state() -> Option<bool> {
    if !available() {
        return None;
    }
    #[cfg(windows)]
    {
        Some(reg::is_registered(&Names::real()))
    }
    #[cfg(not(windows))]
    None
}

/// Which app Win+Shift+S opens now (read only; `None` where the toggle is
/// not offered).
pub fn handler() -> Handler {
    #[cfg(windows)]
    if available() {
        return reg::handler(&Names::real());
    }
    Handler::None
}

/// Public entry for other flows (the tray-off confirmation): which app
/// Win+Shift+S opens now. Same as [`handler`].
pub fn handler_state() -> Handler {
    handler()
}

/// Public entry for other flows (the tray-off confirmation): set up
/// Win+Shift+S. The registration is turned on first when it is off (as
/// the Settings toggle does), then "One more step to use Win+Shift+S"
/// shows — the Settings window opens at Capture with the guide over it
/// (or the open window shows it; nothing over it when Windows already
/// opens Rustshot). Where it is not offered (MSIX, off Windows) nothing
/// happens. `owner` is accepted for callers that have a window; the guide
/// lives in the Settings window, so it is not used.
pub fn show_setup_guide(owner: Option<crate::wind::Hwnd>) {
    let _ = owner;
    match state() {
        None => return,
        Some(false) => {
            if let Err(e) = set(true) {
                let text = tr!("settings.error.screenclip", e);
                eprintln!("rustshot: {text}");
                if !crate::settings_ui::notify(&text) {
                    crate::tray::notify(&text);
                }
                return;
            }
        }
        Some(true) => {}
    }
    crate::settings_ui::show_guide();
}

/// At daemon start: registered, but Windows does not open Rustshot, and
/// the user did not say "Don't show again" — the Settings window opens
/// on the guide. Named (test) instances never do it.
pub fn remind_on_start(hint: bool) {
    if !hint || cfg!(test) || crate::instance::instance_name().is_some() || state() != Some(true) {
        return;
    }
    if handler_state() != Handler::Ours {
        show_setup_guide(None);
    }
}

/// Turn the real registration on or off for this executable.
pub fn set(on: bool) -> Result<(), String> {
    #[cfg(windows)]
    {
        let n = Names::real();
        if on {
            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
            reg::register(&n, &exe)?;
        } else {
            reg::unregister(&n)?;
        }
        reg::notify_shell();
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = on;
        Err("only on Windows".into())
    }
}

/// At daemon start: a registration whose executable moved points here.
/// Named (test) instances never touch it.
pub fn repoint_on_start() {
    #[cfg(windows)]
    {
        if !available() || crate::instance::instance_name().is_some() {
            return;
        }
        let Ok(exe) = std::env::current_exe() else { return };
        match reg::repoint(&Names::real(), &exe) {
            Ok(true) => reg::notify_shell(),
            Ok(false) => {}
            Err(e) => eprintln!("rustshot: could not update the Win+Shift+S handler: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(mode: Mode, delay_s: u32) -> Request {
        Request { mode: Some(mode), delay_s }
    }

    #[test]
    fn legacy_uris() {
        assert_eq!(parse_uri("ms-screenclip:"), req(Mode::Region, 0));
        assert_eq!(parse_uri("ms-screenclip:?source=HotKey"), req(Mode::Region, 0));
        assert_eq!(parse_uri("ms-screenclip:?source=PrintScreen&clippingMode=Rectangle"), req(Mode::Region, 0));
        assert_eq!(parse_uri("ms-screenclip:?clippingMode=Freeform"), req(Mode::Region, 0));
        assert_eq!(parse_uri("ms-screenclip:?clippingMode=Window"), req(Mode::Window, 0));
        assert_eq!(parse_uri("MS-SCREENCLIP:?CLIPPINGMODE=FULLSCREEN"), req(Mode::Full, 0));
        assert_eq!(parse_uri("ms-screenclip:?delayInSeconds=3&clippingMode=Window"), req(Mode::Window, 3));
        assert_eq!(parse_uri("ms-screenclip:?type=recording"), req(Mode::Record, 0));
        assert_eq!(parse_uri("ms-screenclip:?type=recording&clippingMode=Window"), req(Mode::Record, 0));
        assert_eq!(parse_uri("ms-screenclip:?type=snapshot&clippingMode=Window"), req(Mode::Window, 0));
    }

    #[test]
    fn current_uris() {
        let ua = "&user-agent=X&redirect-uri=x-app://r&x-request-correlation-id=1";
        assert_eq!(parse_uri(&format!("ms-screenclip://capture/image?rectangle{ua}")), req(Mode::Region, 0));
        assert_eq!(parse_uri(&format!("ms-screenclip://capture/image?window{ua}")), req(Mode::Window, 0));
        assert_eq!(parse_uri(&format!("ms-screenclip://capture/Image?Freeform{ua}")), req(Mode::Region, 0));
        assert_eq!(
            parse_uri("ms-screenclip://capture/image?rectangle&enabledModes=FullscreenSnip,WindowSnip&auto-save"),
            req(Mode::Region, 0)
        );
        assert_eq!(parse_uri(&format!("ms-screenclip://capture/video?{ua}")), req(Mode::Record, 0));
        assert_eq!(parse_uri("ms-screenclip://capture/video?window"), req(Mode::Record, 0));
        assert_eq!(parse_uri(&format!("ms-screenclip://discover?{ua}")), Request { mode: None, delay_s: 0 });
    }

    /// Quoted as `%1` arrives, odd and hostile input: a region capture
    /// with a bounded delay, never anything else.
    #[test]
    fn untrusted_input() {
        assert_eq!(parse_uri("\"ms-screenclip:?clippingMode=Window\""), req(Mode::Window, 0));
        assert_eq!(parse_uri("  ms-screenclip:?delayInSeconds=99999 "), req(Mode::Region, MAX_DELAY_S));
        assert_eq!(parse_uri("ms-screenclip:?delayInSeconds=-4"), req(Mode::Region, 0));
        assert_eq!(parse_uri("ms-screenclip:?delayInSeconds=1e9"), req(Mode::Region, 0));
        assert_eq!(parse_uri("ms-screenclip:?clippingMode=..%2F..%2Fx&&&=&path=C:\\x"), req(Mode::Region, 0));
        assert_eq!(parse_uri(""), req(Mode::Region, 0));
        assert_eq!(parse_uri("garbage"), req(Mode::Region, 0));
        assert_eq!(parse_uri("rustshot-test-screenclip:?clippingMode=Window"), req(Mode::Window, 0));
    }

    #[test]
    fn slots_match_the_hotkeys() {
        use crate::hotkey::{slot_event, HotEvent};
        assert!(matches!(slot_event(Mode::Region.slot()), HotEvent::Capture));
        assert!(matches!(slot_event(Mode::Window.slot()), HotEvent::CaptureWindow));
        assert!(matches!(slot_event(Mode::Full.slot()), HotEvent::CaptureFull));
        assert!(matches!(slot_event(Mode::Record.slot()), HotEvent::Record));
    }

    #[test]
    fn uri_argument_and_commands() {
        assert!(is_screenclip_uri("ms-screenclip:?source=HotKey"));
        assert!(is_screenclip_uri("\"MS-ScreenClip://capture/image?window\""));
        assert!(!is_screenclip_uri("ms-screenclip"));
        assert!(!is_screenclip_uri("gui"));
        let exe = r"C:\Users\a b\AppData\Local\Programs\rustshot\rustshot.exe";
        let cmd = open_command(exe);
        assert_eq!(cmd, format!("\"{exe}\" screenclip \"%1\""));
        assert_eq!(command_exe(&cmd), Some(exe));
        assert_eq!(command_exe(r"C:\x\rustshot.exe gui"), Some(r"C:\x\rustshot.exe"));
        assert_eq!(command_exe("  "), None);
        assert_eq!(command_exe("\"\" x"), None);
    }

    #[test]
    fn names() {
        let n = Names::real();
        assert_eq!(n.command_key(), r"Software\Classes\Rustshot.ScreenClip\shell\open\command");
        assert_eq!(n.url_associations(), r"Software\Rustshot\Capabilities\URLAssociations");
        assert_eq!(n.user_choice_root(), r"Software\Microsoft\Windows\Shell\Associations\UrlAssociations\ms-screenclip");
    }

    /// A registry in memory: keys (and their parents) and string values,
    /// names case-insensitive as in the real one.
    #[derive(Default)]
    struct FakeReg {
        keys: std::collections::HashSet<(Root, String)>,
        values: std::collections::HashMap<(Root, String, String), String>,
        indirect: std::collections::HashMap<String, String>,
        /// Programs that are gone (every other one is there).
        missing: std::collections::HashSet<String>,
    }

    impl FakeReg {
        fn put(&mut self, root: Root, key: &str, name: Option<&str>, v: &str) -> &mut Self {
            let k = key.to_ascii_lowercase();
            let mut p = k.as_str();
            loop {
                self.keys.insert((root, p.to_string()));
                match p.rfind('\\') {
                    Some(i) => p = &p[..i],
                    None => break,
                }
            }
            self.values.insert((root, k, name.unwrap_or("").to_ascii_lowercase()), v.to_string());
            self
        }

        fn choice(&mut self, latest: Option<&str>, legacy: Option<&str>) -> &mut Self {
            let root = Names::real().user_choice_root();
            if let Some(p) = latest {
                self.put(Root::User, &format!(r"{root}\UserChoiceLatest\ProgId"), Some("ProgId"), p);
            }
            if let Some(p) = legacy {
                self.put(Root::User, &format!(r"{root}\UserChoice"), Some("ProgId"), p);
            }
            self
        }
    }

    impl RegRead for FakeReg {
        fn get(&self, root: Root, key: &str, name: Option<&str>) -> Option<String> {
            self.values.get(&(root, key.to_ascii_lowercase(), name.unwrap_or("").to_ascii_lowercase())).cloned()
        }
        fn exists(&self, root: Root, key: &str) -> bool {
            self.keys.contains(&(root, key.to_ascii_lowercase()))
        }
        fn indirect(&self, s: &str) -> Option<String> {
            self.indirect.get(s).cloned()
        }
        fn file_exists(&self, path: &str) -> bool {
            !self.missing.contains(&path.to_ascii_lowercase())
        }
    }

    /// An app uninstalled but its classic ProgID left behind (the command
    /// names a program that is gone): broken, like a gone AppX. Commands
    /// with variables, bare names or no command are trusted.
    #[test]
    fn stale_classic_progid_is_broken() {
        let n = Names::real();
        let mut r = FakeReg::default();
        r.put(Root::Classes, r"Old.Clip\shell\open\command", None, r#""C:\Program Files\Old\old.exe" "%1""#);
        r.choice(None, Some("Old.Clip"));
        assert!(matches!(handler_in(&r, &n), Handler::Other { .. }));
        r.missing.insert(r"c:\program files\old\old.exe".into());
        assert_eq!(handler_in(&r, &n), Handler::Broken { progid: "Old.Clip".into() });
        r.put(Root::Classes, r"Var.Clip\shell\open\command", None, r#""%ProgramFiles%\Var\var.exe" "%1""#);
        r.put(Root::Classes, r"Bare.Clip\shell\open\command", None, "var.exe %1");
        r.put(Root::Classes, r"Delegate.Clip\shell\open\command", Some("DelegateExecute"), "{GUID}");
        r.missing.insert(r"%programfiles%\var\var.exe".into());
        assert!(resolves(&r, "Var.Clip") && resolves(&r, "Bare.Clip") && resolves(&r, "Delegate.Clip"));
        // Ours with its program moved away: broken (the daemon re-points it).
        let mut ours = FakeReg::default();
        ours.put(Root::User, &n.command_key(), None, &open_command(r"D:\Downloads\rustshot.exe"));
        ours.choice(Some("Rustshot.ScreenClip"), None);
        assert_eq!(handler_in(&ours, &n), Handler::Ours);
        ours.missing.insert(r"d:\downloads\rustshot.exe".into());
        assert_eq!(handler_in(&ours, &n), Handler::Broken { progid: "Rustshot.ScreenClip".into() });
        assert!(is_absolute_windows_path(r"\\srv\share\a.exe") && !is_absolute_windows_path("a.exe"));
    }

    const SNIP: &str = "AppXfeq5vwnakrw6cy02kzhq8ekhhsremh62";

    /// An installed Snipping Tool (its AppX key, an indirect name).
    fn snipping_tool(r: &mut FakeReg) {
        let name = "@{Microsoft.ScreenSketch_11.2409.25.0_x64__8wekyb3d8bbwe?ms-resource://Microsoft.ScreenSketch/Resources/AppName}";
        r.put(Root::Classes, &format!(r"{SNIP}\Application"), Some("ApplicationName"), name);
        r.put(Root::Classes, &format!(r"{SNIP}\Shell\open"), Some("ActivatableClassId"), "App.AppXc99k5qnnsvxj5szemm7fp3g7y08we5vm.mca");
        r.indirect.insert(name.into(), "Snipping Tool".into());
    }

    #[test]
    fn handler_states() {
        let n = Names::real();
        // No choice.
        let mut r = FakeReg::default();
        assert_eq!(handler_in(&r, &n), Handler::None);
        // The Snipping Tool, installed.
        snipping_tool(&mut r);
        r.choice(None, Some(SNIP));
        assert_eq!(handler_in(&r, &n), Handler::Other { progid: SNIP.into(), name: "Snipping Tool".into() });
        // The user's case: the Snipping Tool uninstalled, its choice left.
        let mut gone = FakeReg::default();
        gone.choice(None, Some(SNIP));
        assert_eq!(handler_in(&gone, &n), Handler::Broken { progid: SNIP.into() });
        // Ours (UserChoiceLatest wins over the old UserChoice).
        let mut ours = FakeReg::default();
        ours.put(Root::User, &n.command_key(), None, &open_command(r"C:\x\rustshot.exe"));
        ours.choice(Some("Rustshot.ScreenClip"), Some(SNIP));
        assert_eq!(handler_in(&ours, &n), Handler::Ours);
        assert_eq!(user_choice_in(&ours, &n).as_deref(), Some("Rustshot.ScreenClip"));
        // Our ProgID chosen but our keys gone: broken too.
        let mut stale = FakeReg::default();
        stale.choice(Some("rustshot.screenclip"), None);
        assert_eq!(handler_in(&stale, &n), Handler::Broken { progid: "rustshot.screenclip".into() });
        // An empty choice is no choice.
        let mut empty = FakeReg::default();
        empty.choice(Some("  "), None);
        assert_eq!(handler_in(&empty, &n), Handler::None);
    }

    #[test]
    fn progids_resolve() {
        let mut r = FakeReg::default();
        // A classic handler in the machine's classes (HKCR), one in the user's.
        r.put(Root::Classes, r"ShareX.ScreenClip\shell\open\command", None, r#""C:\Program Files\ShareX\ShareX.exe" "%1""#);
        r.put(Root::User, r"Software\Classes\Greenshot.Clip\shell\open\command", None, r#"C:\gs\Greenshot.exe %1"#);
        assert!(resolves(&r, "ShareX.ScreenClip"));
        assert!(resolves(&r, "sharex.screenclip"), "case-insensitive");
        assert!(resolves(&r, "Greenshot.Clip"));
        // A ProgID key without a command is not a handler (unless AppX).
        r.put(Root::Classes, "Half.Registered", None, "Half");
        assert!(!resolves(&r, "Half.Registered"));
        r.put(Root::Classes, "AppXabc", None, "");
        assert!(resolves(&r, "AppXabc"));
        assert!(!resolves(&r, "AppXmissing"));
        assert!(!resolves(&r, ""));
        assert!(!resolves(&r, r"..\ShareX.ScreenClip"));
        // Names: the description, else the program, else the ProgID.
        assert_eq!(app_name(&r, "ShareX.ScreenClip"), "ShareX");
        r.put(Root::Classes, "ShareX.ScreenClip", None, "ShareX screen capture");
        assert_eq!(app_name(&r, "ShareX.ScreenClip"), "ShareX screen capture");
        r.put(Root::Classes, "ShareX.ScreenClip", None, "URL:ms-screenclip");
        assert_eq!(app_name(&r, "ShareX.ScreenClip"), "ShareX", "a URL: description is no name");
        assert_eq!(app_name(&r, "Greenshot.Clip"), "Greenshot");
        // An indirect name that does not resolve falls through.
        r.put(Root::Classes, r"AppXabc\Application", Some("ApplicationName"), "@{Gone?ms-resource://x}");
        assert_eq!(app_name(&r, "AppXabc"), "AppXabc");
    }

    #[test]
    fn guide_and_state_texts() {
        let plain = guide_text(false);
        assert_eq!(
            plain,
            "Windows lets only you choose which app opens Win+Shift+S.\n1. Click Open Default apps below.\n2. Scroll to the bottom and click \"Choose defaults by link type\".\n3. Find MS-SCREENCLIP and pick Rustshot."
        );
        let broken = guide_text(true);
        assert!(broken.starts_with("Win+Shift+S is currently set to an app that is no longer installed, so Windows shows an error instead of asking.\n"));
        assert!(broken.ends_with(&plain));
        let other = Handler::Other { progid: SNIP.into(), name: "Snipping Tool".into() };
        let broken = Handler::Broken { progid: SNIP.into() };
        assert_eq!(state_line(&Handler::Ours, true), ("Opens Rustshot".into(), Tone::Good));
        assert_eq!(state_line(&other, true), ("Opens Snipping Tool".into(), Tone::Plain));
        assert_eq!(state_line(&broken, true), ("Set to a missing app \u{2014} click to fix".into(), Tone::Bad));
        assert_eq!(state_line(&Handler::None, true), ("Not set".into(), Tone::Plain));
        // The tray icon off: anything but Rustshot leaves no way in.
        let no_way = ("Set up Win+Shift+S \u{2014} otherwise Rustshot can't be opened by a shortcut".to_string(), Tone::Bad);
        for h in [&other, &broken, &Handler::None] {
            assert_eq!(state_line(h, false), no_way);
        }
        assert_eq!(state_line(&Handler::Ours, false), ("Opens Rustshot".into(), Tone::Good));
    }

    /// The plain Default apps page, where the guide's steps apply.
    #[test]
    fn default_apps_page() {
        assert_eq!(DEFAULT_APPS_URI, "ms-settings:defaultapps");
        assert!(GUIDE_STEPS.contains("Choose defaults by link type"));
    }

    /// Register, re-point and unregister a throwaway scheme under HKCU
    /// (never `ms-screenclip`), cleaning up whatever happens.
    #[cfg(windows)]
    mod registry {
        use super::super::reg::*;
        use super::super::{Names, REGISTERED_APPS};
        use std::path::Path;

        struct Throwaway(Names);

        impl Throwaway {
            fn new(tag: &str) -> Throwaway {
                let id = format!("{}{tag}", std::process::id());
                let n = Names {
                    scheme: format!("rustshot-test-screenclip-{id}"),
                    progid: format!("RustshotTest.ScreenClip.{id}"),
                    app: format!("RustshotTest{id}"),
                    app_key: format!(r"Software\RustshotTest{id}"),
                };
                assert_ne!(n.scheme, "ms-screenclip");
                Throwaway(n)
            }
        }

        impl Drop for Throwaway {
            fn drop(&mut self) {
                let n = &self.0;
                let _ = unregister(n);
                let _ = delete_tree(&n.app_key);
                let _ = delete_value(REGISTERED_APPS, &n.app);
                let _ = delete_tree(&n.user_choice_root());
            }
        }

        #[test]
        fn register_and_unregister() {
            let t = Throwaway::new("a");
            let n = &t.0;
            let exe = Path::new(r"C:\Program Files\rustshot\rustshot.exe");
            assert!(!is_registered(n));
            register(n, exe).unwrap();
            assert!(is_registered(n));
            assert_eq!(get(&n.command_key(), None).as_deref(), Some(r#""C:\Program Files\rustshot\rustshot.exe" screenclip "%1""#));
            assert_eq!(get(&n.icon_key(), None).as_deref(), Some(r#""C:\Program Files\rustshot\rustshot.exe",0"#));
            assert_eq!(get(&n.progid_key(), Some("URL Protocol")).as_deref(), Some(""));
            assert_eq!(get(&n.capabilities(), Some("ApplicationName")).as_deref(), Some(n.app.as_str()));
            assert_eq!(get(REGISTERED_APPS, Some(&n.app)), Some(n.capabilities()));
            assert_eq!(registered_exe(n).as_deref(), Some(r"C:\Program Files\rustshot\rustshot.exe"));
            assert!(!is_default(n));
            // Twice is the same.
            register(n, exe).unwrap();
            assert!(is_registered(n));
            unregister(n).unwrap();
            assert!(!is_registered(n));
            assert!(!key_exists(&n.progid_key()));
            assert!(!key_exists(&n.capabilities()));
            assert_eq!(get(REGISTERED_APPS, Some(&n.app)), None);
            // Unregistering what is not there is fine.
            unregister(n).unwrap();
        }

        /// A `RegisteredApplications` value that was not ours comes back.
        #[test]
        fn foreign_value_is_restored() {
            let t = Throwaway::new("b");
            let n = &t.0;
            set(REGISTERED_APPS, Some(&n.app), r"Software\Other\Capabilities").unwrap();
            register(n, Path::new(r"C:\x\rustshot.exe")).unwrap();
            assert!(is_registered(n));
            // A second register does not back up our own value.
            register(n, Path::new(r"C:\x\rustshot.exe")).unwrap();
            unregister(n).unwrap();
            assert_eq!(get(REGISTERED_APPS, Some(&n.app)).as_deref(), Some(r"Software\Other\Capabilities"));
            assert!(!key_exists(&n.backup_key()));
        }

        /// The user's choice is removed only when it is our ProgID.
        #[test]
        fn user_choice_cleanup() {
            let t = Throwaway::new("c");
            let n = &t.0;
            let root = n.user_choice_root();
            register(n, Path::new(r"C:\x\rustshot.exe")).unwrap();
            set(&format!(r"{root}\UserChoice"), Some("ProgId"), &n.progid).unwrap();
            assert!(is_default(n));
            unregister(n).unwrap();
            assert!(!key_exists(&format!(r"{root}\UserChoice")));
            assert!(!is_default(n));

            register(n, Path::new(r"C:\x\rustshot.exe")).unwrap();
            set(&format!(r"{root}\UserChoice"), Some("ProgId"), "AppXsomethingelse").unwrap();
            set(&format!(r"{root}\UserChoiceLatest\ProgId"), Some("ProgId"), &n.progid).unwrap();
            assert!(is_default(n), "UserChoiceLatest wins");
            unregister(n).unwrap();
            assert!(!key_exists(&format!(r"{root}\UserChoiceLatest")));
            assert_eq!(get(&format!(r"{root}\UserChoice"), Some("ProgId")).as_deref(), Some("AppXsomethingelse"));
        }

        /// The live reads: our registration chosen is Ours, a missing
        /// AppX ProgID is Broken, no choice is None.
        #[test]
        fn live_handler_states() {
            use super::super::Handler;
            let t = Throwaway::new("e");
            let n = &t.0;
            let root = n.user_choice_root();
            assert_eq!(handler(n), Handler::None);
            set(&format!(r"{root}\UserChoice"), Some("ProgId"), "AppXrustshotTestDefinitelyMissing0").unwrap();
            assert_eq!(handler(n), Handler::Broken { progid: "AppXrustshotTestDefinitelyMissing0".into() });
            register(n, &std::env::current_exe().unwrap()).unwrap();
            set(&format!(r"{root}\UserChoiceLatest\ProgId"), Some("ProgId"), &n.progid).unwrap();
            assert_eq!(handler(n), Handler::Ours);
            // Its program gone: broken.
            register(n, Path::new(r"C:\definitely\not\here\rustshot.exe")).unwrap();
            assert_eq!(handler(n), Handler::Broken { progid: n.progid.clone() });
        }

        /// A moved exe is re-pointed; another copy that still exists is not.
        #[test]
        fn repoint_follows_a_moved_exe() {
            let t = Throwaway::new("d");
            let n = &t.0;
            let here = std::env::current_exe().unwrap();
            assert!(!repoint(n, &here).unwrap(), "not registered");
            let gone = Path::new(r"C:\definitely\not\here\rustshot.exe");
            register(n, gone).unwrap();
            assert!(repoint(n, &here).unwrap());
            assert_eq!(registered_exe(n).as_deref(), Some(here.display().to_string().as_str()));
            assert!(is_registered(n));
            // Registered to an exe that exists: left alone.
            let other = std::env::temp_dir().join(format!("rustshot-test-{}-other.exe", std::process::id()));
            std::fs::write(&other, b"").unwrap();
            register(n, &other).unwrap();
            assert!(!repoint(n, &here).unwrap());
            assert_eq!(registered_exe(n).as_deref(), Some(other.display().to_string().as_str()));
            let _ = std::fs::remove_file(&other);
        }
    }
}
