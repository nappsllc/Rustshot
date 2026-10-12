#!/usr/bin/env python3
"""A minimal org.kde.StatusNotifierWatcher for the harness (run.sh).

    sni_watcher.py LOG

Owns org.kde.StatusNotifierWatcher on the session bus, keeps the items
that register (dropped when their name goes away) and appends one line
per registration to LOG: "<service> <sender>". It is what a desktop's
tray (KDE Plasma, the GNOME AppIndicator extension, waybar, snixembed)
runs; no icons are drawn.
"""
import sys

import dbus
import dbus.mainloop.glib
import dbus.service
from gi.repository import GLib

IFACE = "org.kde.StatusNotifierWatcher"
PATH = "/StatusNotifierWatcher"
PROPS = "org.freedesktop.DBus.Properties"


class Watcher(dbus.service.Object):
    def __init__(self, bus, log):
        self.log = log
        self.items = []
        self.name = dbus.service.BusName(IFACE, bus, do_not_queue=True)
        super().__init__(self.name, PATH)
        bus.add_signal_receiver(
            self.owner_changed,
            signal_name="NameOwnerChanged",
            dbus_interface="org.freedesktop.DBus",
            bus_name="org.freedesktop.DBus",
            path="/org/freedesktop/DBus",
        )

    @dbus.service.method(IFACE, in_signature="s", out_signature="", sender_keyword="sender")
    def RegisterStatusNotifierItem(self, service, sender=None):
        # A path (libappindicator style) belongs to the sender; a name is the item.
        entry = sender + service if service.startswith("/") else service
        if entry not in self.items:
            self.items.append(entry)
        with open(self.log, "a") as f:
            f.write(f"{entry} {sender}\n")
        print(f"registered {entry} (from {sender})", flush=True)
        self.StatusNotifierItemRegistered(entry)

    @dbus.service.method(IFACE, in_signature="s", out_signature="")
    def RegisterStatusNotifierHost(self, service):
        pass

    @dbus.service.signal(IFACE, signature="s")
    def StatusNotifierItemRegistered(self, service):
        pass

    @dbus.service.signal(IFACE, signature="s")
    def StatusNotifierItemUnregistered(self, service):
        pass

    def owner_changed(self, name, old, new):
        if new:
            return
        for entry in [i for i in self.items if i == name or i.startswith(name + "/")]:
            self.items.remove(entry)
            print(f"unregistered {entry}", flush=True)
            self.StatusNotifierItemUnregistered(entry)

    def props(self):
        return {
            "RegisteredStatusNotifierItems": dbus.Array(self.items, signature="s"),
            "IsStatusNotifierHostRegistered": dbus.Boolean(True),
            "ProtocolVersion": dbus.Int32(0),
        }

    @dbus.service.method(PROPS, in_signature="ss", out_signature="v")
    def Get(self, iface, prop):
        return self.props()[prop]

    @dbus.service.method(PROPS, in_signature="s", out_signature="a{sv}")
    def GetAll(self, iface):
        return self.props()


def main():
    dbus.mainloop.glib.DBusGMainLoop(set_as_default=True)
    bus = dbus.SessionBus()
    Watcher(bus, sys.argv[1])
    print("watcher up", flush=True)
    GLib.MainLoop().run()


if __name__ == "__main__":
    main()
