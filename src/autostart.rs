//! "Start at login" for the tray daemon, per OS (see the plan's autostart locations).
#![cfg_attr(not(windows), allow(dead_code))] // used by the macOS/Linux trays (later tasks)

use std::path::Path;

pub const LABEL: &str = "io.github.nappsllc.rustshot";

/// Windows Run-key value data: `"<exe>" daemon` (`daemon --force` for a
/// forced daemon: it runs with the tray icon off too).
#[cfg_attr(not(windows), allow(dead_code))]
pub fn run_command(exe: &Path, force: bool) -> String {
    format!("\"{}\" daemon{}", exe.display(), if force { " --force" } else { "" })
}

/// A Run value asks for `--force` (kept when the value is written again).
#[cfg_attr(not(windows), allow(dead_code))]
pub fn wants_force(value: &str) -> bool {
    value.split_whitespace().any(|a| a == "--force")
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// macOS LaunchAgent plist: `ProgramArguments` = [exe, "daemon"], `RunAtLoad`.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn launch_agent_plist(exe: &Path) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
<plist version=\"1.0\">\n\
<dict>\n\
\t<key>Label</key>\n\
\t<string>{LABEL}</string>\n\
\t<key>ProgramArguments</key>\n\
\t<array>\n\
\t\t<string>{}</string>\n\
\t\t<string>daemon</string>\n\
\t</array>\n\
\t<key>RunAtLoad</key>\n\
\t<true/>\n\
</dict>\n\
</plist>\n",
        xml_escape(&exe.display().to_string())
    )
}

/// Linux XDG autostart entry (`Exec=<exe> daemon`, quoted per the Desktop Entry spec).
#[cfg_attr(not(all(unix, not(target_os = "macos"))), allow(dead_code))]
pub fn autostart_desktop(exe: &Path) -> String {
    // `%` starts a field code in Exec, so it is always doubled.
    let p = exe.display().to_string().replace('%', "%%");
    let exec = if p.chars().any(|c| c.is_whitespace() || "\"'\\><~|&;$*?#()`".contains(c)) {
        // Quoted argument: backslash-escape `"`, `` ` ``, `$`, `\`; the value is also a
        // string, so every backslash is doubled again (`\` -> 4, `"` -> `\\"`).
        let mut q = String::from("\"");
        for c in p.chars() {
            match c {
                '\\' => q.push_str("\\\\\\\\"),
                '"' | '`' | '$' => {
                    q.push_str("\\\\");
                    q.push(c);
                }
                _ => q.push(c),
            }
        }
        q.push('"');
        q
    } else {
        p
    };
    format!(
        "[Desktop Entry]\nType=Application\nName=Rustshot\nComment=Screenshots, editing and video capture\n\
Exec={exec} daemon\nIcon={LABEL}\nTerminal=false\nX-GNOME-Autostart-enabled=true\n"
    )
}

/// Autostart file location (macOS plist / Linux .desktop) from the relevant env values.
#[cfg_attr(windows, allow(dead_code))]
pub fn autostart_file(home: Option<&str>, xdg_config: Option<&str>, macos: bool) -> Option<std::path::PathBuf> {
    use std::path::PathBuf;
    let ne = |s: Option<&str>| s.filter(|s| !s.is_empty()).map(PathBuf::from);
    if macos {
        Some(ne(home)?.join("Library/LaunchAgents").join(format!("{LABEL}.plist")))
    } else {
        let base = ne(xdg_config).or_else(|| ne(home).map(|h| h.join(".config")))?;
        Some(base.join("autostart").join(format!("{LABEL}.desktop")))
    }
}

/// "Start at login" as the OS has it. Unpackaged it is on or off (the Run
/// value, the LaunchAgent, the autostart file); the Store build's startup
/// task can also be off by the user's hand in Windows (Settings > Apps >
/// Startup, Task Manager), which only they can undo, or fixed by policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    On,
    Off,
    /// Turned off in Windows: the app cannot turn it on again.
    OffByUser,
    /// Set by the organization's policy: shown, not changeable.
    OnByPolicy,
    OffByPolicy,
}

impl Status {
    /// A `Windows.ApplicationModel.StartupTaskState` (0 Disabled,
    /// 1 DisabledByUser, 2 Enabled, 3 DisabledByPolicy, 4 EnabledByPolicy;
    /// anything else reads as off).
    pub fn from_task_state(state: i32) -> Status {
        match state {
            1 => Status::OffByUser,
            2 => Status::On,
            3 => Status::OffByPolicy,
            4 => Status::OnByPolicy,
            _ => Status::Off,
        }
    }

    /// Rustshot starts at login.
    pub fn on(self) -> bool {
        matches!(self, Status::On | Status::OnByPolicy)
    }

    /// Fixed by policy: the switch shows it read-only with [`POLICY`].
    pub fn locked(self) -> bool {
        matches!(self, Status::OnByPolicy | Status::OffByPolicy)
    }
}

/// Why "Start at login" did not change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// Turned off in Windows by the user: [`BY_USER_STEPS`] says where to
    /// turn it on.
    ByUser,
    /// Fixed by policy ([`POLICY`]).
    Policy,
    /// It failed (the error).
    Failed(String),
}

/// How to turn on a startup task the user turned off in Windows.
pub const BY_USER_STEPS: crate::i18n::Key = key!("autostart.by_user");
/// The one-line note of a startup task fixed by policy.
pub const POLICY: crate::i18n::Key = key!("autostart.policy");

impl From<String> for Refusal {
    fn from(e: String) -> Refusal {
        Refusal::Failed(e)
    }
}

/// The state a startup task is in after `on` was asked for: `Ok` when it
/// is what was asked, else why not.
pub fn outcome(on: bool, after: Status) -> Result<(), Refusal> {
    match after {
        s if s.on() == on => Ok(()),
        Status::OffByUser => Err(Refusal::ByUser),
        Status::OnByPolicy | Status::OffByPolicy => Err(Refusal::Policy),
        // Asked for on, still plain off: Windows (or the user, asked) said no.
        _ => Err(Refusal::Failed("Windows did not turn it on".into())),
    }
}

/// "Start at login" here: `None` where it is not offered (Flathub, Snap,
/// the Mac App Store: the store handles it); the Store build's startup
/// task; elsewhere [`is_enabled`].
pub fn status() -> Option<Status> {
    #[cfg(windows)]
    if crate::update::is_msix() {
        return Some(match crate::package_win::startup_state() {
            Ok(s) => Status::from_task_state(s),
            Err(e) => {
                eprintln!("rustshot: could not read the startup task: {e}");
                Status::Off
            }
        });
    }
    if crate::update::managed_install().is_some() {
        return None;
    }
    Some(if is_enabled() { Status::On } else { Status::Off })
}

pub fn is_enabled() -> bool {
    imp::is_enabled()
}

/// Turn "Start at login" on or off ([`status`]'s kind of install).
pub fn set(on: bool) -> Result<(), Refusal> {
    #[cfg(windows)]
    if crate::update::is_msix() {
        let after = crate::package_win::set_startup(on)?;
        return outcome(on, Status::from_task_state(after));
    }
    Ok(imp::set(on)?)
}

#[cfg(windows)]
mod imp {
    use std::path::Path;
    use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
    use windows::Win32::System::Registry::*;
    use windows::core::{HSTRING, PCWSTR, w};

    const VALUE: &str = "rustshot";
    const SUBKEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");

    pub fn is_enabled() -> bool {
        is_enabled_named(VALUE)
    }

    pub fn set(on: bool) -> Result<(), String> {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        set_named(VALUE, on, &exe)
    }

    /// The Run value's data, if any.
    fn read_named(name: &str) -> Option<String> {
        let mut buf = vec![0u16; 2048];
        let mut len = (buf.len() * 2) as u32;
        let r = unsafe {
            RegGetValueW(HKEY_CURRENT_USER, SUBKEY, &HSTRING::from(name), RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr() as *mut _), Some(&mut len))
        };
        (r == ERROR_SUCCESS).then(|| String::from_utf16_lossy(&buf[..(len as usize / 2).saturating_sub(1)]))
    }

    pub fn is_enabled_named(name: &str) -> bool {
        unsafe {
            RegGetValueW(HKEY_CURRENT_USER, SUBKEY, &HSTRING::from(name), RRF_RT_REG_SZ, None, None, None)
                == ERROR_SUCCESS
        }
    }

    pub fn set_named(name: &str, on: bool, exe: &Path) -> Result<(), String> {
        unsafe {
            if on {
                // A forced daemon (this one, or the value already says so) stays forced.
                let force = std::env::args().any(|a| a == "--force") || read_named(name).is_some_and(|v| super::wants_force(&v));
                let data: Vec<u16> = super::run_command(exe, force).encode_utf16().chain(Some(0)).collect();
                let r = RegSetKeyValueW(
                    HKEY_CURRENT_USER,
                    SUBKEY,
                    &HSTRING::from(name),
                    REG_SZ.0,
                    Some(data.as_ptr() as *const _),
                    (data.len() * 2) as u32,
                );
                if r != ERROR_SUCCESS {
                    return Err(format!("registry write failed ({})", r.0));
                }
            } else {
                let r = RegDeleteKeyValueW(HKEY_CURRENT_USER, SUBKEY, &HSTRING::from(name));
                if r != ERROR_SUCCESS && r != ERROR_FILE_NOT_FOUND {
                    return Err(format!("registry delete failed ({})", r.0));
                }
            }
            Ok(())
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn toggle_temp_value() {
            let name = format!("rustshot-test-{}", std::process::id());
            let exe = Path::new("C:\\Program Files\\rustshot\\rustshot.exe");
            assert!(!is_enabled_named(&name));
            set_named(&name, true, exe).unwrap();
            assert!(is_enabled_named(&name));
            set_named(&name, false, exe).unwrap();
            assert!(!is_enabled_named(&name));
            // Deleting a missing value is fine.
            set_named(&name, false, exe).unwrap();
        }
    }
}

#[cfg(unix)]
mod imp {
    pub fn is_enabled() -> bool {
        path().is_some_and(|p| p.exists())
    }

    fn path() -> Option<std::path::PathBuf> {
        super::autostart_file(
            std::env::var("HOME").ok().as_deref(),
            std::env::var("XDG_CONFIG_HOME").ok().as_deref(),
            cfg!(target_os = "macos"),
        )
    }

    pub fn set(on: bool) -> Result<(), String> {
        let p = path().ok_or("cannot locate the autostart directory")?;
        if on {
            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
            let text = if cfg!(target_os = "macos") {
                super::launch_agent_plist(&exe)
            } else {
                super::autostart_desktop(&exe)
            };
            if let Some(dir) = p.parent() {
                std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            }
            std::fs::write(&p, text).map_err(|e| e.to_string())
        } else {
            match std::fs::remove_file(&p) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
                _ => Ok(()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Every StartupTaskState, and what turning it on or off then means.
    #[test]
    fn startup_task_states() {
        let all = [(0, Status::Off), (1, Status::OffByUser), (2, Status::On), (3, Status::OffByPolicy), (4, Status::OnByPolicy), (7, Status::Off), (-1, Status::Off)];
        for (n, want) in all {
            assert_eq!(Status::from_task_state(n), want, "state {n}");
        }
        assert!(Status::On.on() && Status::OnByPolicy.on());
        assert!(!Status::Off.on() && !Status::OffByUser.on() && !Status::OffByPolicy.on());
        assert!(Status::OnByPolicy.locked() && Status::OffByPolicy.locked());
        assert!(!Status::On.locked() && !Status::Off.locked() && !Status::OffByUser.locked());
        // RequestEnableAsync's answers.
        assert_eq!(outcome(true, Status::On), Ok(()));
        assert_eq!(outcome(true, Status::OnByPolicy), Ok(()));
        assert_eq!(outcome(true, Status::OffByUser), Err(Refusal::ByUser));
        assert_eq!(outcome(true, Status::OffByPolicy), Err(Refusal::Policy));
        assert!(matches!(outcome(true, Status::Off), Err(Refusal::Failed(_))));
        // Disable()'s.
        assert_eq!(outcome(false, Status::Off), Ok(()));
        assert_eq!(outcome(false, Status::OffByUser), Ok(()));
        assert_eq!(outcome(false, Status::OnByPolicy), Err(Refusal::Policy));
        assert_eq!(BY_USER_STEPS.get(), "Start at login is turned off in Windows. To turn it on, open Settings > Apps > Startup (or Task Manager > Startup apps) and turn on Rustshot.");
        assert_eq!(POLICY.get(), "Set by your organization");
    }

    #[test]
    fn run_command_quotes_exe() {
        assert_eq!(run_command(Path::new("C:\\a b\\rustshot.exe"), false), "\"C:\\a b\\rustshot.exe\" daemon");
        assert_eq!(run_command(Path::new("C:\\r.exe"), true), "\"C:\\r.exe\" daemon --force");
        assert!(wants_force("\"C:\\r.exe\" daemon --force") && !wants_force("\"C:\\r.exe\" daemon"));
    }

    #[test]
    fn plist_contents() {
        let p = launch_agent_plist(Path::new("/Applications/rust & shot.app/Contents/MacOS/rustshot"));
        assert!(p.contains("<string>io.github.nappsllc.rustshot</string>"));
        assert!(p.contains("<string>/Applications/rust &amp; shot.app/Contents/MacOS/rustshot</string>"));
        assert!(p.contains("<string>daemon</string>"));
        assert!(p.contains("<key>RunAtLoad</key>\n\t<true/>"));
    }

    #[test]
    fn desktop_contents() {
        let d = autostart_desktop(Path::new("/usr/bin/rustshot"));
        assert!(d.contains("Exec=/usr/bin/rustshot daemon\n"));
        assert!(d.starts_with("[Desktop Entry]\n"));
        let q = autostart_desktop(Path::new("/opt/my apps/rustshot"));
        assert!(q.contains("Exec=\"/opt/my apps/rustshot\" daemon\n"));
        let t = autostart_desktop(Path::new("/o p/100%/a\\b"));
        assert!(t.contains("Exec=\"/o p/100%%/a\\\\\\\\b\" daemon\n"), "{t}");
    }

    #[test]
    fn file_locations() {
        assert_eq!(
            autostart_file(Some("/Users/a"), None, true),
            Some(PathBuf::from("/Users/a/Library/LaunchAgents/io.github.nappsllc.rustshot.plist"))
        );
        assert_eq!(
            autostart_file(Some("/home/a"), Some("/cfg"), false),
            Some(PathBuf::from("/cfg/autostart/io.github.nappsllc.rustshot.desktop"))
        );
        assert_eq!(
            autostart_file(Some("/home/a"), Some(""), false),
            Some(PathBuf::from("/home/a/.config/autostart/io.github.nappsllc.rustshot.desktop"))
        );
        assert_eq!(autostart_file(None, None, false), None);
    }
}
