use super::*;
use crate::dbus::{DbusError, Kind, Message, Value};
use crate::tray::RecMenu;
use std::time::Duration;

fn view(rec: Option<RecMenu>) -> View {
    View { recording: rec.is_some(), tip: "Rustshot".into(), menu: tray::menu(Some(true), rec), light_panel: false }
}

/// A call from the host `:1.7` to us.
fn call(path: &str, iface: &str, member: &str, body: Vec<Value>) -> Message {
    let mut m = Message::call(":1.42", path, iface, member, body);
    m.serial = 9;
    m.sender = Some(":1.7".into());
    m
}

/// Through the wire and back: what the host would read.
fn wire(m: Message) -> Message {
    let bytes = m.encode(3).expect("the reply marshals");
    Message::decode(&bytes).expect("and parses").0
}

fn ask(s: &mut Sni, m: Message) -> (Message, Option<Ask>) {
    let (r, a) = s.handle(&m);
    (wire(r.expect("a reply")), a)
}

fn get_all(s: &mut Sni) -> Value {
    let (r, _) = ask(s, call(ITEM_PATH, PROPS, "GetAll", vec![Value::str(ITEM)]));
    assert_eq!(r.kind, Kind::Return);
    assert_eq!(r.reply_to, Some(9));
    assert_eq!(r.dest.as_deref(), Some(":1.7"));
    r.body[0].clone()
}

#[test]
fn item_properties() {
    let mut s = Sni::new(view(None));
    let all = get_all(&mut s);
    for (k, v) in [("Category", "ApplicationStatus"), ("Id", "rustshot"), ("Title", "Rustshot"), ("Status", "Active"), ("Menu", MENU_PATH)] {
        assert_eq!(all.get(k).and_then(Value::as_str), Some(v), "{k}");
    }
    assert_eq!(all.get("ItemIsMenu").and_then(Value::as_bool), Some(false));
    // The tooltip: (icon name, icon pixmaps, title, text).
    let tip = all.get("ToolTip").expect("ToolTip");
    assert_eq!(tip.signature(), "(sa(iiay)ss)");
    assert_eq!(tip.items()[2].as_str(), Some("Rustshot"));
    // One property, as a variant.
    let (r, _) = ask(&mut s, call(ITEM_PATH, PROPS, "Get", vec![Value::str(ITEM), Value::str("Id")]));
    assert_eq!(r.signature(), "v");
    assert_eq!(r.body[0].as_str(), Some("rustshot"));
    // An unknown one is an error; Set is refused.
    let (r, _) = ask(&mut s, call(ITEM_PATH, PROPS, "Get", vec![Value::str(ITEM), Value::str("Nope")]));
    assert_eq!((r.kind, r.error.as_deref()), (Kind::Error, Some("org.freedesktop.DBus.Error.UnknownProperty")));
    let (r, _) = ask(&mut s, call(ITEM_PATH, PROPS, "Set", vec![Value::str(ITEM), Value::str("Id"), Value::variant(Value::str("x"))]));
    assert_eq!(r.kind, Kind::Error);
    // Introspection names the interface and the menu's object.
    let (r, _) = ask(&mut s, call(ITEM_PATH, INTROSPECT, "Introspect", vec![]));
    assert!(r.body[0].as_str().unwrap().contains("org.kde.StatusNotifierItem"));
    let (r, _) = ask(&mut s, call("/", INTROSPECT, "Introspect", vec![]));
    assert!(r.body[0].as_str().unwrap().contains("<node name=\"MenuBar\"/>"));
}

/// The icon: the glyph at 22/32/48 px, ARGB32 big-endian, in the
/// panel's color; the red dot while recording.
#[test]
fn icon_pixmaps_are_argb_in_network_order() {
    let mut s = Sni::new(view(None));
    let all = get_all(&mut s);
    let px = all.get("IconPixmap").expect("IconPixmap");
    assert_eq!(px.signature(), "a(iiay)");
    let sizes: Vec<(i32, i32, usize)> = px
        .items()
        .iter()
        .map(|i| (i.items()[0].as_i32().unwrap(), i.items()[1].as_i32().unwrap(), i.items()[2].items().len()))
        .collect();
    assert_eq!(sizes, [(22, 22, 22 * 22 * 4), (32, 32, 32 * 32 * 4), (48, 48, 48 * 48 * 4)]);
    let centre = |v: &Value, k: usize| -> [u8; 4] {
        let it = &v.items()[k];
        let n = it.items()[0].as_i32().unwrap() as usize;
        let b = it.items()[2].items();
        let i = ((n / 2) * n + n / 2) * 4;
        std::array::from_fn(|j| match b[i + j] {
            Value::Byte(x) => x,
            _ => panic!("bytes"),
        })
    };
    // The centre dot: opaque, white on a dark panel (A, R, G, B).
    assert_eq!(centre(px, 1), [255, 255, 255, 255]);
    assert_eq!(centre(&icon_pixmaps(false, true), 1), [255, 0x1B, 0x1C, 0x20], "dark on a light panel");
    assert_eq!(centre(&icon_pixmaps(true, false), 2), [255, tray::REC_RED.0, tray::REC_RED.1, tray::REC_RED.2]);
    // Corners are transparent.
    let b = px.items()[0].items()[2].items();
    assert_eq!(b[0], Value::Byte(0));
}

fn labels(layout: &Value) -> Vec<(i32, String)> {
    layout.items()[2]
        .items()
        .iter()
        .map(|c| {
            let c = c.inner();
            let id = c.items()[0].as_i32().unwrap();
            let p = &c.items()[1];
            let l = p.get("label").and_then(Value::as_str).map(String::from);
            (id, l.unwrap_or_else(|| p.get("type").and_then(Value::as_str).unwrap_or("?").to_string()))
        })
        .collect()
}

fn get_layout(s: &mut Sni, parent: i32, depth: i32, names: &[&str]) -> Message {
    let names = Value::Array("s".into(), names.iter().map(|n| Value::str(n)).collect());
    let (r, _) = ask(s, call(MENU_PATH, MENU, "GetLayout", vec![Value::I32(parent), Value::I32(depth), names]));
    r
}

#[test]
fn get_layout_is_the_menu_model() {
    let mut s = Sni::new(view(None));
    let r = get_layout(&mut s, 0, -1, &[]);
    assert_eq!(r.signature(), "u(ia{sv}av)");
    assert_eq!(r.body[0], Value::U32(1));
    let root = &r.body[1];
    assert_eq!(root.items()[0], Value::I32(0));
    assert_eq!(root.items()[1].get("children-display").and_then(Value::as_str), Some("submenu"));
    let want: Vec<(i32, String)> = [
        (10, "Capture region"),
        (16, "Capture screen"),
        (17, "Capture window"),
        (11, "Settings…"),
        (12, "Check for updates"),
        (13, "Start at login"),
        (100, "separator"),
        (14, "Quit Rustshot"),
    ]
    .iter()
    .map(|(i, l)| (*i, l.to_string()))
    .collect();
    assert_eq!(labels(root), want);
    // Start at login is a check item, on.
    let auto = root.items()[2].items()[5].inner();
    assert_eq!(auto.items()[1].get("toggle-type").and_then(Value::as_str), Some("checkmark"));
    assert_eq!(auto.items()[1].get("toggle-state").and_then(Value::as_i32), Some(1));
    // Depth 0: the root alone; one item; only the asked properties.
    assert!(get_layout(&mut s, 0, 0, &[]).body[1].items()[2].items().is_empty());
    let one = get_layout(&mut s, 11, -1, &[]);
    assert_eq!(one.body[1].items()[1].get("label").and_then(Value::as_str), Some("Settings…"));
    let only = get_layout(&mut s, 0, -1, &["label"]);
    let first = only.body[1].items()[2].items()[0].inner().items()[1].clone();
    assert!(first.get("label").is_some() && first.get("enabled").is_none());
    // An unknown parent is an error.
    assert_eq!(get_layout(&mut s, 77, -1, &[]).kind, Kind::Error);
    // The menu's own properties.
    let (r, _) = ask(&mut s, call(MENU_PATH, PROPS, "GetAll", vec![Value::str(MENU)]));
    assert_eq!(r.body[0].get("Version").and_then(Value::as_u32), Some(3));
    assert_eq!(r.body[0].get("Status").and_then(Value::as_str), Some("normal"));
}

#[test]
fn layout_while_recording() {
    let rec = RecMenu { paused: true, mic: Some(false), cam: None, pill_hidden: false };
    let mut s = Sni::new(view(Some(rec)));
    let r = get_layout(&mut s, 0, -1, &[]);
    let l: Vec<String> = labels(&r.body[1]).into_iter().map(|(_, l)| l).collect();
    assert_eq!(
        l[..6],
        ["Stop recording", "Resume recording", "Mute microphone", "Hide recording controls", "Discard recording", "separator"]
    );
    // Group properties of two items.
    let ids = Value::Array("i".into(), vec![Value::I32(1), Value::I32(3)]);
    let (r, _) = ask(&mut s, call(MENU_PATH, MENU, "GetGroupProperties", vec![ids, Value::Array("s".into(), vec![])]));
    assert_eq!(r.signature(), "a(ia{sv})");
    let got: Vec<(i32, Option<String>)> =
        r.body[0].items().iter().map(|e| (e.items()[0].as_i32().unwrap(), e.items()[1].get("label").and_then(Value::as_str).map(String::from))).collect();
    assert_eq!(got, [(1, Some("Stop recording".into())), (3, Some("Mute microphone".into()))]);
    let (r, _) = ask(&mut s, call(MENU_PATH, MENU, "GetProperty", vec![Value::I32(3), Value::str("toggle-state")]));
    assert_eq!(r.body[0].as_i32(), Some(0));
}

#[test]
fn events_choose_items() {
    let rec = RecMenu::default();
    let mut s = Sni::new(view(Some(rec)));
    let ev = |id: i32, what: &str| call(MENU_PATH, MENU, "Event", vec![Value::I32(id), Value::str(what), Value::variant(Value::I32(0)), Value::U32(0)]);
    for (id, item) in [(1, MenuItem::StopRecording), (2, MenuItem::Pause(false)), (5, MenuItem::HidePill(false)), (6, MenuItem::Discard), (10, MenuItem::Capture), (14, MenuItem::Quit)] {
        let (r, a) = ask(&mut s, ev(id, "clicked"));
        assert_eq!((r.kind, a), (Kind::Return, Some(Ask::Item(item))), "{id}");
    }
    // Hovering, opening, the root, a separator: nothing.
    assert_eq!(ask(&mut s, ev(10, "hovered")).1, None);
    assert_eq!(ask(&mut s, ev(0, "opened")).1, None);
    assert_eq!(ask(&mut s, ev(SEP_FIRST, "clicked")).1, None);
    // An item not in the menu (no microphone): an error, nothing done.
    let (r, a) = ask(&mut s, ev(3, "clicked"));
    assert_eq!((r.kind, a), (Kind::Error, None));
    // EventGroup: carried out; the unknown ids listed.
    let e = |id: i32| Value::Struct(vec![Value::I32(id), Value::str("clicked"), Value::variant(Value::I32(0)), Value::U32(0)]);
    let group = Value::Array("(isvu)".into(), vec![e(3), e(2)]);
    let (r, a) = ask(&mut s, call(MENU_PATH, MENU, "EventGroup", vec![group]));
    assert_eq!(a, Some(Ask::Item(MenuItem::Pause(false))));
    assert_eq!(r.body[0], Value::Array("i".into(), vec![Value::I32(3)]));
    let (r, _) = ask(&mut s, call(MENU_PATH, MENU, "AboutToShow", vec![Value::I32(0)]));
    assert_eq!(r.body[0], Value::Bool(false));
}

const SEP_FIRST: i32 = tray::SEPARATOR_IDS;

#[test]
fn activate_and_unknown_calls() {
    let mut s = Sni::new(view(None));
    let (r, a) = ask(&mut s, call(ITEM_PATH, ITEM, "Activate", vec![Value::I32(5), Value::I32(5)]));
    assert_eq!((r.kind, a), (Kind::Return, Some(Ask::Activate)));
    assert_eq!(ask(&mut s, call(ITEM_PATH, ITEM, "Scroll", vec![Value::I32(1), Value::str("vertical")])).1, None);
    let (r, _) = ask(&mut s, call(ITEM_PATH, ITEM, "Explode", vec![]));
    assert_eq!(r.error.as_deref(), Some("org.freedesktop.DBus.Error.UnknownMethod"));
    let (r, _) = ask(&mut s, call("/elsewhere", ITEM, "Activate", vec![]));
    assert_eq!(r.error.as_deref(), Some("org.freedesktop.DBus.Error.UnknownObject"));
    // A caller that wants no reply gets none (the action still happens).
    let mut m = call(ITEM_PATH, ITEM, "Activate", vec![Value::I32(0), Value::I32(0)]);
    m.flags = crate::dbus::NO_REPLY;
    let (r, a) = s.handle(&m);
    assert!(r.is_none());
    assert_eq!(a, Some(Ask::Activate));
}

#[test]
fn changes_are_signalled() {
    let mut s = Sni::new(view(None));
    assert!(s.update(view(None)).is_empty(), "nothing changed");
    // Recording: the menu, the icon and the tip change.
    let mut v = view(Some(RecMenu::default()));
    v.tip = "Rustshot — recording 0:00".into();
    let sig = s.update(v.clone());
    let names: Vec<(String, String)> = sig.iter().map(|m| (m.path.clone().unwrap(), m.member.clone().unwrap())).collect();
    assert_eq!(
        names,
        [
            (ITEM_PATH.into(), "NewIcon".into()),
            (ITEM_PATH.into(), "NewToolTip".into()),
            (MENU_PATH.into(), "LayoutUpdated".into())
        ]
    );
    assert_eq!(sig[2].body, [Value::U32(2), Value::I32(0)]);
    assert!(sig.iter().all(|m| m.kind == Kind::Signal && m.dest.is_none()));
    for m in sig {
        wire(m);
    }
    assert_eq!(s.revision(), 2);
    // The layout now has the new revision and the tip the time.
    assert_eq!(get_layout(&mut s, 0, -1, &[]).body[0], Value::U32(2));
    assert_eq!(get_all(&mut s).get("ToolTip").unwrap().items()[2].as_str(), Some("Rustshot — recording 0:00"));
    // Paused: only the menu (one item's label).
    v.menu = tray::menu(Some(true), Some(RecMenu { paused: true, ..RecMenu::default() }));
    let sig = s.update(v.clone());
    assert_eq!(sig.len(), 1);
    assert_eq!(sig[0].member.as_deref(), Some("LayoutUpdated"));
    // A light panel: only the icon.
    v.light_panel = true;
    assert_eq!(s.update(v).iter().map(|m| m.member.clone().unwrap()).collect::<Vec<_>>(), ["NewIcon"]);
}

/// A scripted session bus: the bus's own methods and the watcher.
struct Fake {
    /// RequestName's answer (1 primary owner; an error when None).
    request: Option<u32>,
    watcher: bool,
    calls: Vec<Message>,
}

impl Bus for Fake {
    fn unique_name(&self) -> &str {
        ":1.42"
    }

    fn call(&mut self, m: Message, _timeout: Duration) -> anyhow::Result<Message> {
        let member = m.member.clone().unwrap_or_default();
        let dest = m.dest.clone().unwrap_or_default();
        self.calls.push(Message::call(&dest, m.path.as_deref().unwrap_or(""), m.iface.as_deref().unwrap_or(""), &member, m.body.clone()));
        let err = |name: &str| Err(anyhow::Error::new(DbusError { name: name.into(), text: String::new() }));
        match (dest.as_str(), member.as_str()) {
            (crate::dbus::BUS_NAME, "RequestName") => match self.request {
                Some(code) => Ok(Message::reply(&m, vec![Value::U32(code)])),
                None => err("org.freedesktop.DBus.Error.AccessDenied"),
            },
            (crate::dbus::BUS_NAME, "ReleaseName") => Ok(Message::reply(&m, vec![Value::U32(1)])),
            (crate::dbus::BUS_NAME, "NameHasOwner") => Ok(Message::reply(&m, vec![Value::Bool(self.watcher)])),
            (WATCHER, "RegisterStatusNotifierItem") if self.watcher => Ok(Message::reply(&m, vec![])),
            _ => err("org.freedesktop.DBus.Error.ServiceUnknown"),
        }
    }

    fn send(&mut self, _m: Message) -> anyhow::Result<u32> {
        Ok(0)
    }

    fn wait_signal(&mut self, _want: &dyn Fn(&Message) -> bool, _timeout: Duration) -> anyhow::Result<Option<Message>> {
        Ok(None)
    }
}

#[test]
fn registration_with_the_watcher() {
    assert_eq!(item_name(4242), "org.kde.StatusNotifierItem-4242-1");
    // The name is ours: registered by it.
    let mut b = Fake { request: Some(1), watcher: true, calls: Vec::new() };
    let name = claim_name(&mut b, &item_name(7));
    assert_eq!(name, "org.kde.StatusNotifierItem-7-1");
    assert!(register(&mut b, &name).unwrap());
    let reg = b.calls.last().unwrap();
    assert_eq!((reg.path.as_deref(), reg.iface.as_deref()), (Some(WATCHER_PATH), Some(WATCHER)));
    assert_eq!(reg.body, [Value::str("org.kde.StatusNotifierItem-7-1")]);
    assert_eq!(b.calls[0].body, [Value::str("org.kde.StatusNotifierItem-7-1"), Value::U32(4)], "DO_NOT_QUEUE");
    // Refused (a sandbox) or taken: by the unique name.
    for request in [None, Some(3)] {
        let mut b = Fake { request, watcher: true, calls: Vec::new() };
        assert_eq!(claim_name(&mut b, &item_name(7)), ":1.42");
    }
    // No watcher: not registered, no error.
    let mut b = Fake { request: Some(1), watcher: false, calls: Vec::new() };
    assert!(!register(&mut b, "org.kde.StatusNotifierItem-7-1").unwrap());
    assert!(!b.calls.iter().any(|m| m.member.as_deref() == Some("RegisterStatusNotifierItem")));
    // Releasing the name (hidden icon); a unique name is never released.
    release_name(&mut b, ":1.42");
    assert!(!b.calls.iter().any(|m| m.member.as_deref() == Some("ReleaseName")));
    release_name(&mut b, "org.kde.StatusNotifierItem-7-1");
    assert_eq!(b.calls.last().unwrap().member.as_deref(), Some("ReleaseName"));
}

#[test]
fn the_watcher_coming_and_going() {
    let noc = |name: &str, old: &str, new: &str| {
        let mut m = Message::signal(crate::dbus::BUS_PATH, crate::dbus::BUS_NAME, "NameOwnerChanged", vec![Value::str(name), Value::str(old), Value::str(new)]);
        m.sender = Some(crate::dbus::BUS_NAME.into());
        wire(m)
    };
    assert_eq!(watcher_change(&noc(WATCHER, "", ":1.9")), Some(true));
    assert_eq!(watcher_change(&noc(WATCHER, ":1.9", "")), Some(false));
    assert_eq!(watcher_change(&noc("org.other", "", ":1.9")), None);
    assert!(watcher_rule().contains("arg0='org.kde.StatusNotifierWatcher'"));
}

#[test]
fn panel_colour() {
    assert!(!light_panel("KDE", None), "no preference: dark");
    assert!(!light_panel("KDE", Some(1)));
    assert!(light_panel("KDE", Some(2)));
    assert!(light_panel("XFCE", Some(2)));
    assert!(!light_panel("ubuntu:GNOME", Some(2)), "GNOME's top bar is dark");
    let mut m = Message::signal(PORTAL_PATH, SETTINGS, "SettingChanged", vec![
        Value::str("org.freedesktop.appearance"),
        Value::str("color-scheme"),
        Value::variant(Value::U32(2)),
    ]);
    m.sender = Some(":1.3".into());
    assert_eq!(scheme_change(&wire(m)), Some(2));
    let other = Message::signal(PORTAL_PATH, SETTINGS, "SettingChanged", vec![Value::str("org.gnome"), Value::str("x"), Value::variant(Value::U32(2))]);
    assert_eq!(scheme_change(&other), None);
}
