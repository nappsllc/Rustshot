//! The Linux tray icon: runs the StatusNotifierItem (`sni`) on its own
//! session-bus connection, on a thread of its own.
//!
//! The item registers with `org.kde.StatusNotifierWatcher` and registers
//! again whenever a watcher appears (the panel restarted). No watcher (a
//! desktop without a tray, e.g. GNOME without the AppIndicator extension):
//! no icon, said once on stderr; the daemon, its hotkeys and a second
//! launch work as ever. Hiding the icon (`show_tray` off) gives the name
//! up, so the watcher drops it. A broken connection is made again.

use crate::dbus::{self, Bus, Conn, Kind};
use crate::hotkey::HotEvent;
use crate::sni::{self, Ask, Sni, View};
use crate::tray;
use anyhow::Result;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

/// How long one wait for the bus lasts (the tray's changes are looked at
/// in between).
const TICK: Duration = Duration::from_millis(200);
/// Before connecting again after the connection broke.
const RECONNECT: Duration = Duration::from_secs(5);

/// Start the tray thread; events go to `tx`.
pub fn spawn(tx: Sender<HotEvent>) {
    let spawned = std::thread::Builder::new().name("rustshot-tray".into()).spawn(move || run(tx));
    if let Err(e) = spawned {
        eprintln!("rustshot: no tray icon: {e}");
    }
}

fn run(tx: Sender<HotEvent>) {
    let mut told = false;
    loop {
        let conn = match Conn::session() {
            Ok(c) => c,
            Err(e) => {
                eprintln!("rustshot: no tray icon: {e:#}");
                return;
            }
        };
        match session(conn, &tx, &mut told) {
            Ok(()) => return,
            Err(e) => eprintln!("rustshot: tray: {e:#}; connecting again"),
        }
        std::thread::sleep(RECONNECT);
    }
}

/// The color scheme's panel color (the desktop from `XDG_CURRENT_DESKTOP`).
fn light(scheme: Option<u32>) -> bool {
    sni::light_panel(&std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default(), scheme)
}

/// Serve the item on `c` until the connection breaks (`Err`) or the
/// daemon is gone (`Ok`).
fn session(mut c: Conn, tx: &Sender<HotEvent>, told: &mut bool) -> Result<()> {
    dbus::add_match(&mut c, &sni::watcher_rule())?;
    let _ = dbus::add_match(&mut c, &sni::scheme_rule());
    let mut scheme = sni::color_scheme(&mut c);
    let mut sni = Sni::new(View::current(light(scheme)));
    let wanted = sni::item_name(std::process::id());
    // The name we go by while the icon is shown, and whether the watcher has us.
    let mut owned: Option<String> = None;
    let mut register = false;
    let mut generation = tray::generation();
    let mut last_view = Instant::now();
    loop {
        match (tray::is_visible(), &owned) {
            (true, None) => {
                owned = Some(sni::claim_name(&mut c, &wanted));
                register = true;
            }
            (false, Some(n)) => {
                sni::release_name(&mut c, n);
                owned = None;
            }
            _ => {}
        }
        if register && let Some(name) = &owned {
            register = false;
            match sni::register(&mut c, name) {
                Ok(true) => *told = false,
                Ok(false) if !*told => {
                    *told = true;
                    eprintln!("rustshot: no tray on this desktop (no StatusNotifierWatcher on the session bus); the icon shows once one starts");
                }
                Ok(false) => {}
                Err(e) => eprintln!("rustshot: tray: could not register the icon: {e:#}"),
            }
        }
        if let Some(m) = c.wait_any(TICK)? {
            match m.kind {
                Kind::Call => {
                    // The menu is read as it is now (Start at login may have
                    // changed in Settings meanwhile).
                    if m.path.as_deref() == Some(sni::MENU_PATH) {
                        for s in sni.update(View::current(light(scheme))) {
                            c.send(s)?;
                        }
                    }
                    let (reply, ask) = sni.handle(&m);
                    if let Some(r) = reply {
                        c.send(r)?;
                    }
                    match ask {
                        Some(Ask::Activate) => {
                            if tx.send(tray::click_event()).is_err() {
                                return Ok(()); // the daemon is gone
                            }
                        }
                        Some(Ask::Item(item)) => {
                            if let Err(e) = tray::activate(item, tx) {
                                tray::notify(&e);
                            }
                            generation = generation.wrapping_sub(1); // look again now
                        }
                        None => {}
                    }
                }
                Kind::Signal => {
                    if let Some(appeared) = sni::watcher_change(&m) {
                        register = appeared;
                    } else if let Some(s) = sni::scheme_change(&m) {
                        scheme = Some(s);
                        generation = generation.wrapping_sub(1);
                    }
                }
                _ => {}
            }
        }
        // What the tray shows changed (or a while passed: Start at login).
        let g = tray::generation();
        if g != generation || last_view.elapsed() > Duration::from_secs(5) {
            generation = g;
            last_view = Instant::now();
            for s in sni.update(View::current(light(scheme))) {
                c.send(s)?;
            }
        }
    }
}

/// The tray against the live session bus with a watcher (the Docker
/// harness starts `docker/linux-test/sni_watcher.py`): registers, answers
/// GetLayout, and registers again when the watcher comes back.
#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "needs a session bus and a StatusNotifierWatcher"]
    fn live_sni_tray_registers() {
        use super::*;
        if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_none() {
            println!("SKIP: no session bus");
            return;
        }
        let mut probe = Conn::session().expect("session bus");
        let has = dbus::bus_call(&mut probe, "NameHasOwner", vec![dbus::Value::str(sni::WATCHER)]).unwrap();
        if has.body[0].as_bool() != Some(true) {
            println!("SKIP: no StatusNotifierWatcher");
            return;
        }
        let (tx, _rx) = std::sync::mpsc::channel();
        spawn(tx);
        let name = sni::item_name(std::process::id());
        let until = Instant::now() + Duration::from_secs(10);
        let mut items = String::new();
        while Instant::now() < until {
            let v = dbus::get_property(&mut probe, sni::WATCHER, sni::WATCHER_PATH, sni::WATCHER, "RegisteredStatusNotifierItems").unwrap();
            items = format!("{v:?}");
            if items.contains(&name) {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(items.contains(&name), "registered: {items}");
        let m = dbus::Message::call(
            &name,
            sni::MENU_PATH,
            sni::MENU,
            "GetLayout",
            vec![dbus::Value::I32(0), dbus::Value::I32(-1), dbus::Value::Array("s".into(), vec![])],
        );
        let r = probe.call(m, dbus::CALL_TIMEOUT).unwrap();
        let text = format!("{:?}", r.body);
        assert!(text.contains("Capture") && text.contains("Quit Rustshot"), "{text}");
        println!("SNI registered as {name}; GetLayout ok");
    }
}
