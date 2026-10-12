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
        let key = HSTRING::from(key);
        let name = name.map(HSTRING::from);
        let pname = name.as_ref().map_or(PCWSTR::null(), |n| PCWSTR(n.as_ptr()));
        unsafe {
            let mut len = 0u32;
            let r = RegGetValueW(HKEY_CURRENT_USER, &key, pname, RRF_RT_REG_SZ, None, None, Some(&mut len));
            if r != ERROR_SUCCESS {
                return None;
            }
            let mut buf = vec![0u16; (len as usize).div_ceil(2) + 1];
            let mut len = (buf.len() * 2) as u32;
            let r = RegGetValueW(
                HKEY_CURRENT_USER,
                &key,
                pname,
                RRF_RT_REG_SZ,
                None,
                Some(buf.as_mut_ptr().cast()),
                Some(&mut len),
            );
            if r != ERROR_SUCCESS {
                return None;
            }
            let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
            Some(String::from_utf16_lossy(&buf[..n]))
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
        let mut h = HKEY::default();
        let r = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, &HSTRING::from(key), None, KEY_READ, &mut h) };
        if r == ERROR_SUCCESS {
            unsafe {
                let _ = RegCloseKey(h);
            }
            true
        } else {
            false
        }
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

    /// The ProgID the user chose for the scheme: `UserChoiceLatest`'s
    /// (newer Windows 11) when there is one, else `UserChoice`'s.
    pub fn user_choice(n: &Names) -> Option<String> {
        let root = n.user_choice_root();
        get(&format!(r"{root}\UserChoiceLatest\ProgId"), Some("ProgId"))
            .or_else(|| get(&format!(r"{root}\UserChoice"), Some("ProgId")))
    }

    /// Windows opens the scheme with us.
    pub fn is_default(n: &Names) -> bool {
        user_choice(n).is_some_and(|p| p.eq_ignore_ascii_case(&n.progid))
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

/// Registered, but Windows still opens the Snipping Tool (the user has
/// not picked Rustshot yet).
pub fn awaiting_choice() -> bool {
    #[cfg(windows)]
    {
        let n = Names::real();
        available() && reg::is_registered(&n) && !reg::is_default(&n)
    }
    #[cfg(not(windows))]
    false
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
