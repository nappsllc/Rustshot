#!/usr/bin/env python3
"""Drive org.freedesktop.portal.ScreenCast without a user: CreateSession,
SelectSources (one monitor), Start; print the PipeWire node ids and hold the
session open for HOLD seconds (so `pw-cli ls Node` can see the stream), then
close it. Exit 0 when Start returned at least one stream.

Usage: portal_screencast.py [HOLD_SECONDS] [READY_FILE]
READY_FILE (optional) gets the node ids once the stream is up.
"""
import os
import sys

import dbus
import dbus.mainloop.glib
from gi.repository import GLib

HOLD = float(sys.argv[1]) if len(sys.argv) > 1 else 5.0
READY = sys.argv[2] if len(sys.argv) > 2 else None

dbus.mainloop.glib.DBusGMainLoop(set_as_default=True)
bus = dbus.SessionBus()
portal = bus.get_object("org.freedesktop.portal.Desktop", "/org/freedesktop/portal/desktop")
sc = dbus.Interface(portal, "org.freedesktop.portal.ScreenCast")
loop = GLib.MainLoop()
sender = bus.get_unique_name()[1:].replace(".", "_")
state = {"n": 0, "session": None, "rc": 1}


def log(*a):
    print("PORTAL", *a, flush=True)


def request(method, *args, options):
    state["n"] += 1
    token = "rs%d" % state["n"]
    path = "/org/freedesktop/portal/desktop/request/%s/%s" % (sender, token)
    options = dict(options)
    options["handle_token"] = token
    done = {}

    def on_response(code, results):
        done["r"] = (int(code), results)
        loop.quit()

    bus.add_signal_receiver(on_response, "Response", "org.freedesktop.portal.Request", path=path)
    method(*args, dbus.Dictionary(options, signature="sv"))
    timer = GLib.timeout_add_seconds(20, loop.quit)
    loop.run()
    GLib.source_remove(timer)
    if "r" not in done:
        raise SystemExit("PORTAL FAIL: no Response to %s within 20 s" % method._method_name)
    code, results = done["r"]
    log(method._method_name, "response", code, {str(k): v for k, v in results.items()})
    if code != 0:
        raise SystemExit("PORTAL FAIL: %s answered %d" % (method._method_name, code))
    return results


try:
    log("version", portal.Get("org.freedesktop.portal.ScreenCast", "version",
                              dbus_interface="org.freedesktop.DBus.Properties"))
    log("AvailableSourceTypes", portal.Get("org.freedesktop.portal.ScreenCast", "AvailableSourceTypes",
                                           dbus_interface="org.freedesktop.DBus.Properties"))
    r = request(sc.CreateSession, options={"session_handle_token": "rssession"})
    session = dbus.ObjectPath(str(r["session_handle"]))
    state["session"] = session
    request(sc.SelectSources, session, options={"types": dbus.UInt32(1), "multiple": False,
                                                "cursor_mode": dbus.UInt32(int(os.environ.get("RS_CURSOR_MODE", "2")))})
    r = request(sc.Start, session, "", options={})
    streams = r.get("streams", [])
    nodes = [int(s[0]) for s in streams]
    for node, props in streams:
        log("stream node=%d" % node, {str(k): v for k, v in props.items()})
    if not nodes:
        raise SystemExit("PORTAL FAIL: Start returned no streams")
    fd = sc.OpenPipeWireRemote(session, dbus.Dictionary({}, signature="sv")).take()
    log("OpenPipeWireRemote fd", fd)
    png = os.environ.get("RS_FRAME_PNG")
    if png:
        # One picture through the portal's remote, the way OBS reads it:
        # GStreamer's pipewiresrc on that fd, the stream's node as the target.
        import subprocess
        cmd = ["gst-launch-1.0", "-q", "pipewiresrc", "fd=%d" % fd, "path=%d" % nodes[0],
               "always-copy=true", "num-buffers=5", "!", "videoconvert", "!",
               "pngenc", "snapshot=true", "!", "filesink", "location=" + png]
        try:
            r = subprocess.run(cmd, pass_fds=[fd], timeout=15, capture_output=True, text=True)
            log("gst rc", r.returncode, (r.stdout + r.stderr).strip()[-400:])
        except subprocess.TimeoutExpired as e:
            out = (e.stdout or b"") + (e.stderr or b"")
            if isinstance(out, bytes):
                out = out.decode(errors="replace")
            log("gst timed out after 15 s (no picture)", out.strip()[-1500:])
    log("NODES", " ".join(map(str, nodes)))
    if READY:
        with open(READY, "w") as f:
            f.write(" ".join(map(str, nodes)) + "\n")
    GLib.timeout_add(int(HOLD * 1000), loop.quit)
    loop.run()
    state["rc"] = 0
finally:
    if state["session"] is not None:
        try:
            bus.get_object("org.freedesktop.portal.Desktop", state["session"]).Close(
                dbus_interface="org.freedesktop.portal.Session")
            log("session closed")
        except dbus.DBusException as e:
            log("close failed:", e)
sys.exit(state["rc"])
