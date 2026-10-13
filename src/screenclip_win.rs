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
/// classes or the merged ones), or it is a packaged app's ProgID that is
/// still registered (an installed AppX package keeps its `AppX…` key
/// under `HKCR`; uninstalling the package removes it).
pub fn resolves(r: &impl RegRead, progid: &str) -> bool {
    if progid.is_empty() || progid.contains(['\\', '/']) {
        return false;
    }
    r.exists(Root::User, &format!(r"Software\Classes\{progid}\shell\open\command"))
        || r.exists(Root::Classes, &format!(r"{progid}\shell\open\command"))
        || (is_appx(progid) && r.exists(Root::Classes, progid))
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
    if let Some(stem) = cmd.as_deref().and_then(command_exe).and_then(|e| std::path::Path::new(e).file_stem()) {
        return stem.to_string_lossy().into_owned();
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
pub const GUIDE_TITLE: &str = "One more step to use Win+Shift+S";
pub const GUIDE_STEPS: &str = "Windows lets only you choose which app opens Win+Shift+S.\n\
1. Click Open Default apps below.\n\
2. Scroll to the bottom and click \"Choose defaults by link type\".\n\
3. Find MS-SCREENCLIP and pick Rustshot.";
/// The steps when the button opens Rustshot's own Default apps page
/// ([`default_apps_uri`] on a Windows that has it): no link-type list to
/// scroll to.
pub const GUIDE_STEPS_APP_PAGE: &str = "Windows lets only you choose which app opens Win+Shift+S.\n\
1. Click Open Default apps below.\n\
2. Click the box under MS-SCREENCLIP.\n\
3. Pick Rustshot and click Set default.";
/// Said first when the choice names an app that is gone.
pub const GUIDE_BROKEN: &str =
    "Win+Shift+S is currently set to an app that is no longer installed, so Windows shows an error instead of asking.";
pub const GUIDE_OPEN: &str = "Open Default apps";
pub const GUIDE_LATER: &str = "Remind me later";
pub const GUIDE_DONT_SHOW: &str = "Don't show again";
/// The toast once Windows uses Rustshot.
pub const NOW_OURS: &str = "Win+Shift+S now opens Rustshot";

/// The guide's body for a choice that is `broken` or not; `app_page`:
/// the button opens Rustshot's own page ([`opens_app_page`]).
pub fn guide_text(broken: bool, app_page: bool) -> String {
    let steps = if app_page { GUIDE_STEPS_APP_PAGE } else { GUIDE_STEPS };
    if broken { format!("{GUIDE_BROKEN}\n{steps}") } else { steps.to_string() }
}

/// How a handler line reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Good,
    Plain,
    Bad,
}

/// The Settings row's line for `h`.
pub fn state_line(h: &Handler) -> (String, Tone) {
    match h {
        Handler::Ours => ("Opens Rustshot".into(), Tone::Good),
        Handler::Other { name, .. } => (format!("Opens {name}"), Tone::Plain),
        Handler::Broken { .. } => ("Set to a missing app \u{2014} click to fix".into(), Tone::Bad),
        Handler::None => ("Not set".into(), Tone::Plain),
    }
}

/// The Settings page to send the user to. Windows 11 21H2 / 22H2 with the
/// April 2023 update (22000.1817, 22621.1555) and later open an app's own
/// Default apps page for `ms-settings:defaultapps?registeredAppUser=<its
/// RegisteredApplications name>`; older builds get the plain Default apps
/// page (where "Choose defaults by link type" is).
pub fn default_apps_uri(build: u32, ubr: u32, app: &str) -> String {
    let plain = !app.is_empty() && app.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'));
    if has_app_page(build, ubr) && plain {
        format!("ms-settings:defaultapps?registeredAppUser={app}")
    } else {
        "ms-settings:defaultapps".into()
    }
}

/// This Windows build opens an app's own Default apps page.
pub fn has_app_page(build: u32, ubr: u32) -> bool {
    match build {
        b if b >= 22631 => true,
        22621..=22630 => ubr >= 1555,
        22000..=22620 => ubr >= 1817,
        _ => false,
    }
}

/// The Windows build and update revision (`CurrentBuildNumber`, `UBR`);
/// zeros where they cannot be read (and off Windows).
pub fn windows_build() -> (u32, u32) {
    #[cfg(windows)]
    {
        reg::windows_build()
    }
    #[cfg(not(windows))]
    (0, 0)
}

/// The Open Default apps button goes to Rustshot's own page here.
pub fn opens_app_page() -> bool {
    let (b, u) = windows_build();
    has_app_page(b, u)
}

/// Open Windows' Default apps (Rustshot's page where the build has it).
pub fn open_default_apps() {
    #[cfg(windows)]
    {
        let (b, u) = windows_build();
        let uri = default_apps_uri(b, u, &Names::real().app);
        reg::shell_open(&uri);
    }
}

/// The registry, HKCU only.
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

    /// `CurrentBuildNumber` and `UBR` of this Windows (zeros if unread).
    pub fn windows_build() -> (u32, u32) {
        const KEY: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";
        let build = get_in(HKEY_LOCAL_MACHINE, KEY, Some("CurrentBuildNumber"), RRF_RT_REG_SZ)
            .and_then(|b| b.trim().parse().ok())
            .unwrap_or(0);
        let mut ubr = 0u32;
        let mut len = 4u32;
        let r = unsafe {
            RegGetValueW(HKEY_LOCAL_MACHINE, &HSTRING::from(KEY), &HSTRING::from("UBR"), RRF_RT_REG_DWORD, None, Some((&raw mut ubr).cast()), Some(&mut len))
        };
        (build, if r == ERROR_SUCCESS { ubr } else { 0 })
    }

    /// Open a `ms-settings:` page.
    pub fn shell_open(uri: &str) {
        use windows::Win32::UI::Shell::ShellExecuteW;
        use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
        if !uri.starts_with("ms-settings:") {
            return;
        }
        unsafe {
            ShellExecuteW(None, &HSTRING::from("open"), &HSTRING::from(uri), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
        }
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

/// At daemon start: registered, but Windows does not open Rustshot, and
/// the user did not say "Don't show again" — the Settings window opens
/// on the guide. Named (test) instances never do it.
pub fn remind_on_start(hint: bool) {
    if !hint || cfg!(test) || crate::instance::instance_name().is_some() || state() != Some(true) {
        return;
    }
    if handler() != Handler::Ours {
        crate::settings_ui::show_guide();
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
        let plain = guide_text(false, false);
        assert_eq!(
            plain,
            "Windows lets only you choose which app opens Win+Shift+S.\n1. Click Open Default apps below.\n2. Scroll to the bottom and click \"Choose defaults by link type\".\n3. Find MS-SCREENCLIP and pick Rustshot."
        );
        let broken = guide_text(true, false);
        assert!(broken.starts_with("Win+Shift+S is currently set to an app that is no longer installed, so Windows shows an error instead of asking.\n"));
        assert!(broken.ends_with(&plain));
        assert!(guide_text(false, true).contains("MS-SCREENCLIP") && !guide_text(false, true).contains("Scroll"));
        assert!(guide_text(true, true).starts_with(GUIDE_BROKEN));
        assert_eq!(state_line(&Handler::Ours), ("Opens Rustshot".into(), Tone::Good));
        assert_eq!(state_line(&Handler::Other { progid: SNIP.into(), name: "Snipping Tool".into() }), ("Opens Snipping Tool".into(), Tone::Plain));
        assert_eq!(state_line(&Handler::Broken { progid: SNIP.into() }), ("Set to a missing app \u{2014} click to fix".into(), Tone::Bad));
        assert_eq!(state_line(&Handler::None), ("Not set".into(), Tone::Plain));
    }

    #[test]
    fn default_apps_page() {
        let plain = "ms-settings:defaultapps";
        let ours = "ms-settings:defaultapps?registeredAppUser=Rustshot";
        assert_eq!(default_apps_uri(19045, 5000, "Rustshot"), plain, "Windows 10");
        assert_eq!(default_apps_uri(22000, 1816, "Rustshot"), plain);
        assert_eq!(default_apps_uri(22000, 1817, "Rustshot"), ours);
        assert_eq!(default_apps_uri(22621, 1554, "Rustshot"), plain);
        assert_eq!(default_apps_uri(22621, 1555, "Rustshot"), ours);
        assert_eq!(default_apps_uri(22631, 0, "Rustshot"), ours);
        assert_eq!(default_apps_uri(26300, 9457, "Rustshot"), ours);
        assert_eq!(default_apps_uri(26300, 0, "a b&c"), plain, "only a plain name goes in the URI");
        assert_eq!(default_apps_uri(0, 0, "Rustshot"), plain, "unknown build");
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
            register(n, Path::new(r"C:\x\rustshot.exe")).unwrap();
            set(&format!(r"{root}\UserChoiceLatest\ProgId"), Some("ProgId"), &n.progid).unwrap();
            assert_eq!(handler(n), Handler::Ours);
            let (build, _) = super::super::windows_build();
            assert!(build >= 10240, "a Windows 10+ build number: {build}");
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
