//! The Linux tray icon as a StatusNotifierItem (KDE/freedesktop SNI) with
//! a `com.canonical.dbusmenu` menu, over the hand-written D-Bus client
//! (`dbus`). This module is the protocol (pure, tested on every platform
//! against a scripted bus): the objects' properties, method calls and
//! signals, the menu layout from the shared model (`tray::menu`), and the
//! registration with `org.kde.StatusNotifierWatcher`. The thread that runs
//! it on the session bus is `tray_linux`.
//!
//! - The item is `/StatusNotifierItem` (interface
//!   `org.kde.StatusNotifierItem`), the menu `/MenuBar`.
//! - The name is `org.kde.StatusNotifierItem-<pid>-1`; where it cannot be
//!   owned (a Flatpak sandbox allows only its app id) the item registers
//!   under its unique name (`:1.42`), which the watchers accept too.
//! - The icon is a pixmap (ARGB32, network byte order) of the tray glyph at
//!   22, 32 and 48 px, white on a dark panel and near-black on a light one;
//!   while recording, its centre is the red dot.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))] // the tray runs on Linux

use crate::dbus::{self, Bus, Kind, Message, Value};
use crate::tray::{self, MenuItem};
use anyhow::Result;

pub const ITEM_PATH: &str = "/StatusNotifierItem";
pub const MENU_PATH: &str = "/MenuBar";
pub const ITEM: &str = "org.kde.StatusNotifierItem";
pub const MENU: &str = "com.canonical.dbusmenu";
pub const WATCHER: &str = "org.kde.StatusNotifierWatcher";
pub const WATCHER_PATH: &str = "/StatusNotifierWatcher";
const PROPS: &str = "org.freedesktop.DBus.Properties";
const INTROSPECT: &str = "org.freedesktop.DBus.Introspectable";
const PEER: &str = "org.freedesktop.DBus.Peer";
/// The icon sizes sent (the host picks the closest).
pub const ICON_SIZES: [u32; 3] = [22, 32, 48];
/// The glyph's color on a dark panel and on a light one.
const ON_DARK: (u8, u8, u8) = (255, 255, 255);
const ON_LIGHT: (u8, u8, u8) = (0x1B, 0x1C, 0x20);

/// What the tray shows now.
#[derive(Clone, Debug, PartialEq)]
pub struct View {
    pub recording: bool,
    /// The tooltip's title ("Rustshot — recording 1:23").
    pub tip: String,
    pub menu: Vec<MenuItem>,
    /// The panel is light: a dark glyph.
    pub light_panel: bool,
}

impl View {
    /// The tray's state as the daemon published it.
    pub fn current(light_panel: bool) -> View {
        View { recording: tray::is_recording(), tip: tray::tip_text(), menu: tray::current_menu(), light_panel }
    }
}

/// What a call asked of the app.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ask {
    /// The icon was clicked (`Activate`): capture, or stop the recording.
    Activate,
    /// A menu item was chosen.
    Item(MenuItem),
}

/// The exported objects: the item and its menu.
pub struct Sni {
    view: View,
    /// The menu layout's revision (bumped when the menu changes).
    revision: u32,
}

/// The name the item asks for: `org.kde.StatusNotifierItem-<pid>-1`.
pub fn item_name(pid: u32) -> String {
    format!("{ITEM}-{pid}-1")
}

impl Sni {
    pub fn new(view: View) -> Sni {
        Sni { view, revision: 1 }
    }

    #[cfg(test)]
    pub fn revision(&self) -> u32 {
        self.revision
    }

    /// Take `view`; the signals that say what changed (`NewIcon`,
    /// `NewToolTip`, the menu's `LayoutUpdated`).
    pub fn update(&mut self, view: View) -> Vec<Message> {
        let mut out = Vec::new();
        if view.recording != self.view.recording || view.light_panel != self.view.light_panel {
            out.push(Message::signal(ITEM_PATH, ITEM, "NewIcon", vec![]));
        }
        if view.tip != self.view.tip {
            out.push(Message::signal(ITEM_PATH, ITEM, "NewToolTip", vec![]));
        }
        if view.menu != self.view.menu {
            self.revision = self.revision.wrapping_add(1);
            out.push(Message::signal(MENU_PATH, MENU, "LayoutUpdated", vec![Value::U32(self.revision), Value::I32(0)]));
        }
        self.view = view;
        out
    }

    /// Answer the method call `m`: the reply (none when the caller wants
    /// none) and what it asked of the app.
    pub fn handle(&mut self, m: &Message) -> (Option<Message>, Option<Ask>) {
        let (reply, ask) = match self.answer(m) {
            Ok((body, ask)) => (Message::reply(m, body), ask),
            Err((name, text)) => (Message::error_reply(m, name, &text), None),
        };
        ((m.flags & dbus::NO_REPLY == 0).then_some(reply), ask)
    }

    fn answer(&mut self, m: &Message) -> Result<(Vec<Value>, Option<Ask>), Fault> {
        let path = m.path.as_deref().unwrap_or("");
        let iface = m.iface.as_deref().unwrap_or("");
        let member = m.member.as_deref().unwrap_or("");
        let arg = |i: usize| m.body.get(i);
        let str_arg = |i: usize| arg(i).and_then(Value::as_str).unwrap_or("").to_string();
        let int_arg = |i: usize| arg(i).and_then(Value::as_i32).unwrap_or(0);
        match (iface, member) {
            (PEER, "Ping") => return Ok((vec![], None)),
            (PEER, "GetMachineId") => return Ok((vec![Value::str(&machine_id())], None)),
            (INTROSPECT, "Introspect") | ("", "Introspect") => return Ok((vec![Value::str(&introspect(path))], None)),
            _ => {}
        }
        match path {
            ITEM_PATH => match (iface, member) {
                (PROPS, "Get") if str_arg(0) == ITEM => {
                    let p = str_arg(1);
                    self.item_prop(&p).map(|v| (vec![Value::variant(v)], None)).ok_or_else(|| unknown_prop(&p))
                }
                (PROPS, "GetAll") if str_arg(0) == ITEM || str_arg(0).is_empty() => {
                    let all = ITEM_PROPS.iter().filter_map(|p| self.item_prop(p).map(|v| (*p, v))).collect();
                    Ok((vec![Value::dict_sv(all)], None))
                }
                (PROPS, "GetAll") => Ok((vec![Value::dict_sv(vec![])], None)),
                (PROPS, "Set") => Err(("org.freedesktop.DBus.Error.PropertyReadOnly", "read-only".into())),
                (ITEM | "", "Activate") => Ok((vec![], Some(Ask::Activate))),
                (ITEM | "", "SecondaryActivate" | "ContextMenu" | "Scroll" | "ProvideXdgActivationToken") => Ok((vec![], None)),
                _ => Err(unknown_method(iface, member)),
            },
            MENU_PATH => match (iface, member) {
                (PROPS, "Get") if str_arg(0) == MENU => {
                    let p = str_arg(1);
                    menu_prop(&p).map(|v| (vec![Value::variant(v)], None)).ok_or_else(|| unknown_prop(&p))
                }
                (PROPS, "GetAll") if str_arg(0) == MENU || str_arg(0).is_empty() => {
                    let all = MENU_PROPS.iter().filter_map(|p| menu_prop(p).map(|v| (*p, v))).collect();
                    Ok((vec![Value::dict_sv(all)], None))
                }
                (PROPS, "GetAll") => Ok((vec![Value::dict_sv(vec![])], None)),
                (PROPS, "Set") => Err(("org.freedesktop.DBus.Error.PropertyReadOnly", "read-only".into())),
                (MENU | "", "GetLayout") => {
                    let names = string_list(arg(2));
                    let layout = self.layout(int_arg(0), int_arg(1), &names).ok_or_else(|| bad_id(int_arg(0)))?;
                    Ok((vec![Value::U32(self.revision), layout], None))
                }
                (MENU | "", "GetGroupProperties") => {
                    let ids: Vec<i32> = arg(0).map(|v| v.items().iter().filter_map(Value::as_i32).collect()).unwrap_or_default();
                    let names = string_list(arg(1));
                    let items = self
                        .entries()
                        .into_iter()
                        .filter(|(id, _)| ids.is_empty() || ids.contains(id))
                        .map(|(id, it)| Value::Struct(vec![Value::I32(id), props_dict(item_props(it), &names)]))
                        .collect();
                    Ok((vec![Value::Array("(ia{sv})".into(), items)], None))
                }
                (MENU | "", "GetProperty") => {
                    let (id, name) = (int_arg(0), str_arg(1));
                    let props = if id == 0 { root_props() } else { item_props(self.item(id).ok_or_else(|| bad_id(id))?) };
                    let v = props.into_iter().find(|(k, _)| *k == name).map(|(_, v)| v).ok_or_else(|| unknown_prop(&name))?;
                    Ok((vec![Value::variant(v)], None))
                }
                (MENU | "", "Event") => {
                    let id = int_arg(0);
                    let ask = self.event(id, &str_arg(1))?;
                    Ok((vec![], ask))
                }
                (MENU | "", "EventGroup") => {
                    // a(isvu): the first click is carried out; unknown ids are listed.
                    let mut ask = None;
                    let mut errors = Vec::new();
                    for e in arg(0).map(Value::items).unwrap_or_default() {
                        let f = e.items();
                        let id = f.first().and_then(Value::as_i32).unwrap_or(-1);
                        let ev = f.get(1).and_then(Value::as_str).unwrap_or("");
                        match self.event(id, ev) {
                            Ok(a) => ask = ask.or(a),
                            Err(_) => errors.push(Value::I32(id)),
                        }
                    }
                    Ok((vec![Value::Array("i".into(), errors)], ask))
                }
                (MENU | "", "AboutToShow") => Ok((vec![Value::Bool(false)], None)),
                (MENU | "", "AboutToShowGroup") => Ok((vec![Value::Array("i".into(), vec![]), Value::Array("i".into(), vec![])], None)),
                _ => Err(unknown_method(iface, member)),
            },
            _ => Err(("org.freedesktop.DBus.Error.UnknownObject", format!("no object at {path}"))),
        }
    }

    /// A menu event: "clicked" on an item chooses it; the others are fine.
    fn event(&self, id: i32, ev: &str) -> Result<Option<Ask>, Fault> {
        if id == 0 {
            return Ok(None); // the root: opened, closed
        }
        let it = self.item(id).ok_or_else(|| bad_id(id))?;
        Ok((ev == "clicked" && it != MenuItem::Separator).then_some(Ask::Item(it)))
    }

    /// The menu's items with their ids.
    fn entries(&self) -> Vec<(i32, MenuItem)> {
        tray::with_ids(&self.view.menu)
    }

    fn item(&self, id: i32) -> Option<MenuItem> {
        self.entries().into_iter().find(|(i, _)| *i == id).map(|(_, it)| it)
    }

    /// `GetLayout`: the subtree at `parent` (`depth` -1 = all; 0 = the
    /// node alone), with the properties in `names` (empty = all).
    fn layout(&self, parent: i32, depth: i32, names: &[String]) -> Option<Value> {
        let node = |id: i32, props: Vec<(&'static str, Value)>, kids: Vec<Value>| {
            Value::Struct(vec![Value::I32(id), props_dict(props, names), Value::Array("v".into(), kids)])
        };
        // The submenus ("Recent uploads"): each top item with its own.
        let entries = self.entries();
        let tree = tray::nest(&self.view.menu);
        let sub = |kids: &[usize], depth: i32| -> Vec<Value> {
            if depth == 0 {
                return Vec::new();
            }
            kids.iter().map(|k| Value::variant(node(entries[*k].0, item_props(entries[*k].1), vec![]))).collect()
        };
        let with_sub = |i: usize, kids: &[usize], depth: i32| {
            let (id, it) = entries[i];
            let mut props = item_props(it);
            if !kids.is_empty() {
                props.push(("children-display", Value::str("submenu")));
            }
            node(id, props, sub(kids, depth))
        };
        if parent == 0 {
            let kids = if depth == 0 {
                Vec::new()
            } else {
                tree.iter().map(|(i, kids)| Value::variant(with_sub(*i, kids, depth - 1))).collect()
            };
            return Some(node(0, root_props(), kids));
        }
        let i = entries.iter().position(|(id, _)| *id == parent)?;
        let kids = tree.iter().find(|(p, _)| *p == i).map(|(_, k)| k.clone()).unwrap_or_default();
        Some(with_sub(i, &kids, depth))
    }

    /// The item's property `p`.
    fn item_prop(&self, p: &str) -> Option<Value> {
        let s = Value::str;
        Some(match p {
            "Category" => s("ApplicationStatus"),
            "Id" => s("rustshot"),
            "Title" => s("Rustshot"),
            "Status" => s("Active"),
            "WindowId" => Value::I32(0),
            "IconName" | "OverlayIconName" | "AttentionIconName" | "AttentionMovieName" | "IconThemePath" => s(""),
            "IconPixmap" => icon_pixmaps(self.view.recording, self.view.light_panel),
            "OverlayIconPixmap" | "AttentionIconPixmap" => Value::Array("(iiay)".into(), vec![]),
            "ToolTip" => Value::Struct(vec![s(""), Value::Array("(iiay)".into(), vec![]), s(&self.view.tip), s("")]),
            "ItemIsMenu" => Value::Bool(false),
            "Menu" => Value::path(MENU_PATH),
            _ => return None,
        })
    }
}

/// The item's properties (GetAll's order).
const ITEM_PROPS: [&str; 16] = [
    "Category",
    "Id",
    "Title",
    "Status",
    "WindowId",
    "IconName",
    "IconPixmap",
    "OverlayIconName",
    "OverlayIconPixmap",
    "AttentionIconName",
    "AttentionIconPixmap",
    "AttentionMovieName",
    "ToolTip",
    "ItemIsMenu",
    "Menu",
    "IconThemePath",
];

const MENU_PROPS: [&str; 4] = ["Version", "TextDirection", "Status", "IconThemePath"];

fn menu_prop(p: &str) -> Option<Value> {
    Some(match p {
        "Version" => Value::U32(3),
        "TextDirection" => Value::str("ltr"),
        "Status" => Value::str("normal"),
        "IconThemePath" => Value::Array("s".into(), vec![]),
        _ => return None,
    })
}

fn root_props() -> Vec<(&'static str, Value)> {
    vec![("children-display", Value::str("submenu"))]
}

/// A menu item's dbusmenu properties.
fn item_props(it: MenuItem) -> Vec<(&'static str, Value)> {
    if it == MenuItem::Separator {
        return vec![("type", Value::str("separator")), ("visible", Value::Bool(true))];
    }
    let mut p = vec![("label", Value::str(&it.label())), ("enabled", Value::Bool(true)), ("visible", Value::Bool(true))];
    if let Some(on) = it.checked() {
        p.push(("toggle-type", Value::str("checkmark")));
        p.push(("toggle-state", Value::I32(i32::from(on))));
    }
    p
}

/// `props` as an `a{sv}`, only those in `names` (empty = all).
fn props_dict(props: Vec<(&'static str, Value)>, names: &[String]) -> Value {
    Value::dict_sv(props.into_iter().filter(|(k, _)| names.is_empty() || names.iter().any(|n| n == k)).collect())
}

fn string_list(v: Option<&Value>) -> Vec<String> {
    v.map(|v| v.items().iter().filter_map(|s| s.as_str().map(String::from)).collect()).unwrap_or_default()
}

/// An error reply's name and text.
type Fault = (&'static str, String);

fn unknown_method(iface: &str, member: &str) -> Fault {
    ("org.freedesktop.DBus.Error.UnknownMethod", format!("no method {iface}.{member}"))
}

fn unknown_prop(p: &str) -> Fault {
    ("org.freedesktop.DBus.Error.UnknownProperty", format!("no property {p}"))
}

fn bad_id(id: i32) -> Fault {
    ("org.freedesktop.DBus.Error.InvalidArgs", format!("no menu item {id}"))
}

fn machine_id() -> String {
    std::fs::read_to_string("/etc/machine-id").map(|s| s.trim().to_string()).unwrap_or_default()
}

/// The glyph as `a(iiay)`: ARGB32 in network byte order at [`ICON_SIZES`].
pub fn icon_pixmaps(recording: bool, light_panel: bool) -> Value {
    let rgb = if light_panel { ON_LIGHT } else { ON_DARK };
    let icons = ICON_SIZES
        .iter()
        .map(|&n| {
            let rgba = tray::tray_glyph_rgba_rec(n, rgb, recording);
            let mut argb = Vec::with_capacity(rgba.len());
            for p in rgba.as_chunks::<4>().0 {
                argb.extend([p[3], p[0], p[1], p[2]].map(Value::Byte));
            }
            Value::Struct(vec![Value::I32(n as i32), Value::I32(n as i32), Value::Array("y".into(), argb)])
        })
        .collect();
    Value::Array("(iiay)".into(), icons)
}

/// Introspection data of the object at `path`.
fn introspect(path: &str) -> String {
    let head = "<!DOCTYPE node PUBLIC \"-//freedesktop//DTD D-BUS Object Introspection 1.0//EN\" \"http://www.freedesktop.org/standards/dbus/1.0/introspect.dtd\">\n<node>\n";
    let std_ifaces = r#"  <interface name="org.freedesktop.DBus.Introspectable"><method name="Introspect"><arg type="s" direction="out"/></method></interface>
  <interface name="org.freedesktop.DBus.Peer"><method name="Ping"/><method name="GetMachineId"><arg type="s" direction="out"/></method></interface>
  <interface name="org.freedesktop.DBus.Properties">
    <method name="Get"><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="v" direction="out"/></method>
    <method name="GetAll"><arg type="s" direction="in"/><arg type="a{sv}" direction="out"/></method>
    <method name="Set"><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="v" direction="in"/></method>
    <signal name="PropertiesChanged"><arg type="s"/><arg type="a{sv}"/><arg type="as"/></signal>
  </interface>
"#;
    let body = match path {
        ITEM_PATH => r#"  <interface name="org.kde.StatusNotifierItem">
    <property name="Category" type="s" access="read"/>
    <property name="Id" type="s" access="read"/>
    <property name="Title" type="s" access="read"/>
    <property name="Status" type="s" access="read"/>
    <property name="WindowId" type="i" access="read"/>
    <property name="IconName" type="s" access="read"/>
    <property name="IconPixmap" type="a(iiay)" access="read"/>
    <property name="OverlayIconName" type="s" access="read"/>
    <property name="OverlayIconPixmap" type="a(iiay)" access="read"/>
    <property name="AttentionIconName" type="s" access="read"/>
    <property name="AttentionIconPixmap" type="a(iiay)" access="read"/>
    <property name="AttentionMovieName" type="s" access="read"/>
    <property name="ToolTip" type="(sa(iiay)ss)" access="read"/>
    <property name="ItemIsMenu" type="b" access="read"/>
    <property name="Menu" type="o" access="read"/>
    <property name="IconThemePath" type="s" access="read"/>
    <method name="ContextMenu"><arg name="x" type="i" direction="in"/><arg name="y" type="i" direction="in"/></method>
    <method name="Activate"><arg name="x" type="i" direction="in"/><arg name="y" type="i" direction="in"/></method>
    <method name="SecondaryActivate"><arg name="x" type="i" direction="in"/><arg name="y" type="i" direction="in"/></method>
    <method name="Scroll"><arg name="delta" type="i" direction="in"/><arg name="orientation" type="s" direction="in"/></method>
    <signal name="NewTitle"/>
    <signal name="NewIcon"/>
    <signal name="NewAttentionIcon"/>
    <signal name="NewOverlayIcon"/>
    <signal name="NewToolTip"/>
    <signal name="NewStatus"><arg name="status" type="s"/></signal>
  </interface>
"#
        .to_string(),
        MENU_PATH => r#"  <interface name="com.canonical.dbusmenu">
    <property name="Version" type="u" access="read"/>
    <property name="TextDirection" type="s" access="read"/>
    <property name="Status" type="s" access="read"/>
    <property name="IconThemePath" type="as" access="read"/>
    <method name="GetLayout"><arg type="i" direction="in"/><arg type="i" direction="in"/><arg type="as" direction="in"/><arg type="u" direction="out"/><arg type="(ia{sv}av)" direction="out"/></method>
    <method name="GetGroupProperties"><arg type="ai" direction="in"/><arg type="as" direction="in"/><arg type="a(ia{sv})" direction="out"/></method>
    <method name="GetProperty"><arg type="i" direction="in"/><arg type="s" direction="in"/><arg type="v" direction="out"/></method>
    <method name="Event"><arg type="i" direction="in"/><arg type="s" direction="in"/><arg type="v" direction="in"/><arg type="u" direction="in"/></method>
    <method name="EventGroup"><arg type="a(isvu)" direction="in"/><arg type="ai" direction="out"/></method>
    <method name="AboutToShow"><arg type="i" direction="in"/><arg type="b" direction="out"/></method>
    <method name="AboutToShowGroup"><arg type="ai" direction="in"/><arg type="ai" direction="out"/><arg type="ai" direction="out"/></method>
    <signal name="ItemsPropertiesUpdated"><arg type="a(ia{sv})"/><arg type="a(ias)"/></signal>
    <signal name="LayoutUpdated"><arg type="u"/><arg type="i"/></signal>
    <signal name="ItemActivationRequested"><arg type="i"/><arg type="u"/></signal>
  </interface>
"#
        .to_string(),
        _ => "  <node name=\"StatusNotifierItem\"/>\n  <node name=\"MenuBar\"/>\n".to_string(),
    };
    format!("{head}{std_ifaces}{body}</node>\n")
}

// ---------------------------------------------------------------------------
// Names and the watcher
// ---------------------------------------------------------------------------

/// `RequestName` flag: fail instead of queueing for a taken name.
const DO_NOT_QUEUE: u32 = 4;

/// Own the item's name `name`; when that is refused (a sandbox), the
/// item goes by our unique name. Returns the name to register.
pub fn claim_name(bus: &mut dyn Bus, name: &str) -> String {
    match dbus::bus_call(bus, "RequestName", vec![Value::str(name), Value::U32(DO_NOT_QUEUE)]) {
        // 1: the primary owner now, 4: already it.
        Ok(r) if matches!(r.body.first().and_then(Value::as_u32), Some(1 | 4)) => name.to_string(),
        _ => bus.unique_name().to_string(),
    }
}

/// Give up `name` (the icon is hidden): the watcher drops the item.
pub fn release_name(bus: &mut dyn Bus, name: &str) {
    if !name.starts_with(':') {
        let _ = dbus::bus_call(bus, "ReleaseName", vec![Value::str(name)]);
    }
}

/// The match rule that tells us when the watcher comes or goes.
pub fn watcher_rule() -> String {
    format!("type='signal',sender='{}',interface='{}',member='NameOwnerChanged',arg0='{WATCHER}'", dbus::BUS_NAME, dbus::BUS_NAME)
}

/// Register the item `service` with the watcher. `Ok(false)`: there is
/// no watcher (no tray on this desktop, for now).
pub fn register(bus: &mut dyn Bus, service: &str) -> Result<bool> {
    let has = dbus::bus_call(bus, "NameHasOwner", vec![Value::str(WATCHER)])?;
    if has.body.first().and_then(Value::as_bool) != Some(true) {
        return Ok(false);
    }
    let m = Message::call(WATCHER, WATCHER_PATH, WATCHER, "RegisterStatusNotifierItem", vec![Value::str(service)]);
    bus.call(m, dbus::CALL_TIMEOUT)?;
    Ok(true)
}

/// A `NameOwnerChanged` of the watcher: `Some(true)` when one appeared
/// (register again), `Some(false)` when it went away.
pub fn watcher_change(m: &Message) -> Option<bool> {
    if m.kind != Kind::Signal || !m.is(dbus::BUS_NAME, "NameOwnerChanged") {
        return None;
    }
    let [name, _old, new] = m.body.as_slice() else { return None };
    (name.as_str() == Some(WATCHER)).then(|| new.as_str().is_some_and(|s| !s.is_empty()))
}

// ---------------------------------------------------------------------------
// The panel's color
// ---------------------------------------------------------------------------

const PORTAL: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const SETTINGS: &str = "org.freedesktop.portal.Settings";

/// Whether the panel is light, from the desktop (`XDG_CURRENT_DESKTOP`)
/// and the color scheme (`org.freedesktop.appearance color-scheme`:
/// 1 dark, 2 light, 0 no preference). GNOME's top bar is dark in both
/// schemes; elsewhere the panel follows the scheme (no preference: dark).
pub fn light_panel(desktop: &str, scheme: Option<u32>) -> bool {
    let gnome = desktop.split(':').any(|d| matches!(d.to_ascii_lowercase().as_str(), "gnome" | "ubuntu" | "unity" | "pantheon"));
    !gnome && scheme == Some(2)
}

/// The color scheme through the Settings portal (None: no portal).
pub fn color_scheme(bus: &mut dyn Bus) -> Option<u32> {
    let args = || vec![Value::str("org.freedesktop.appearance"), Value::str("color-scheme")];
    let one = Message::call(PORTAL, PORTAL_PATH, SETTINGS, "ReadOne", args());
    if let Ok(r) = bus.call(one, dbus::CALL_TIMEOUT) {
        return r.body.first().and_then(Value::as_u32);
    }
    // Portals before 1.18 have only Read (a variant in a variant).
    let r = bus.call(Message::call(PORTAL, PORTAL_PATH, SETTINGS, "Read", args()), dbus::CALL_TIMEOUT).ok()?;
    r.body.first().and_then(Value::as_u32)
}

/// The match rule for color-scheme changes.
pub fn scheme_rule() -> String {
    format!("type='signal',interface='{SETTINGS}',member='SettingChanged',path='{PORTAL_PATH}',arg0='org.freedesktop.appearance'")
}

/// A `SettingChanged` of the color scheme: its new value.
pub fn scheme_change(m: &Message) -> Option<u32> {
    if m.kind != Kind::Signal || !m.is(SETTINGS, "SettingChanged") {
        return None;
    }
    let [ns, key, v] = m.body.as_slice() else { return None };
    (ns.as_str() == Some("org.freedesktop.appearance") && key.as_str() == Some("color-scheme")).then(|| v.as_u32()).flatten()
}

#[cfg(test)]
mod tests;
