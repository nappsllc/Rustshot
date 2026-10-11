//! xdg-desktop-portal clients over the hand-written D-Bus client
//! ([`crate::dbus`]): ScreenCast (Wayland, and Flatpak on any session) and
//! Camera.
//!
//! ScreenCast: `CreateSession` → `SelectSources` (one monitor, the cursor
//! embedded, `persist_mode` 2 with the stored restore token where the
//! portal has version 4+) → `Start` (the portal's dialog: the user picks the
//! screen) → `OpenPipeWireRemote` (a PipeWire socket as a descriptor) and
//! the stream's node id, position and size. Every request's `Response`
//! signal is subscribed to *before* the call, on the object path computed
//! from our unique name and the `handle_token` (no race with a fast
//! portal); an older portal that answers with another path is followed
//! there. The session stays open (and the stream with it) while the
//! [`Cast`] lives; dropping it closes the session.
//!
//! Screenshot: `Screenshot` without the dialog (`interactive` false; a
//! desktop may still ask once whether Rustshot may take screenshots): a
//! PNG file's URI, read and removed by [`grab_screen`]. On Wayland this is
//! the only way to see the screen (X11 `GetImage` of the XWayland root
//! fails with BadMatch).
//!
//! Camera: `IsCameraPresent`, `AccessCamera` (asks the user once; the
//! permission store remembers), then `OpenPipeWireRemote`, on which only
//! cameras are visible.
//!
//! The restore token lives in `$XDG_STATE_HOME/rustshot/screencast-token`
//! (mode 0600), replaced by the one each `Start` returns.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))] // Linux; the pure parts are tested everywhere

use crate::capture::IRect;
use crate::dbus::{self, Bus, Fd, Message, Value};
use anyhow::{anyhow, Context, Result};
use std::path::PathBuf;
use std::time::Duration;

pub const DESKTOP: &str = "org.freedesktop.portal.Desktop";
pub const DESKTOP_PATH: &str = "/org/freedesktop/portal/desktop";
pub const SCREENCAST: &str = "org.freedesktop.portal.ScreenCast";
pub const CAMERA: &str = "org.freedesktop.portal.Camera";
pub const SCREENSHOT: &str = "org.freedesktop.portal.Screenshot";
const REQUEST: &str = "org.freedesktop.portal.Request";
const SESSION: &str = "org.freedesktop.portal.Session";

/// How long a request without a dialog may take.
const QUICK: Duration = Duration::from_secs(20);
/// How long the user may take in a portal dialog.
const DIALOG: Duration = Duration::from_secs(300);
/// How long `Start` with a stored restore token may take: a restored
/// choice comes back at once. Longer, and the token is dropped and the
/// dialog asked afresh (a portal that no longer knows the token).
const RESTORED: Duration = Duration::from_secs(15);
/// How often a waiting request looks whether it was called off.
const CANCEL_POLL: Duration = Duration::from_millis(100);

/// ScreenCast source types and cursor modes.
const MONITOR: u32 = 1;
const CURSOR_HIDDEN: u32 = 1;
const CURSOR_EMBEDDED: u32 = 2;
/// Persist until revoked.
const PERSIST: u32 = 2;

/// The user closed the portal dialog (nothing to report as a failure).
#[derive(Debug)]
pub struct Cancelled;

impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("cancelled in the portal dialog")
    }
}

impl std::error::Error for Cancelled {}

/// The portal did not answer a request in time (its dialog was closed).
#[derive(Debug)]
pub struct NoAnswer {
    pub method: String,
    pub secs: u64,
}

impl std::fmt::Display for NoAnswer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: the portal did not answer within {} s", self.method, self.secs)
    }
}

impl std::error::Error for NoAnswer {}

/// The portal said no (response 2: refused, or an error behind it).
#[derive(Debug)]
pub struct Refused {
    pub method: String,
    pub code: u32,
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: the portal refused (response {})", self.method, self.code)
    }
}

impl std::error::Error for Refused {}

/// Set from another thread to call a waiting request off (its dialog is
/// closed and the request ends as [`Cancelled`]).
pub type CancelFlag = std::sync::atomic::AtomicBool;

fn cancelled(c: Option<&CancelFlag>) -> bool {
    c.is_some_and(|c| c.load(std::sync::atomic::Ordering::SeqCst))
}

/// Whether capture must go through the portal: a Wayland session (XShm of
/// the X root under XWayland sees only X clients) or a Flatpak sandbox.
pub fn needs_portal(wayland_display: Option<&std::ffi::OsStr>, session_type: Option<&str>, flatpak: bool) -> bool {
    flatpak || !crate::rec_ui::x11_session(wayland_display, session_type)
}

/// [`needs_portal`] for this process.
#[cfg(target_os = "linux")]
pub fn needs_portal_now() -> bool {
    needs_portal(std::env::var_os("WAYLAND_DISPLAY").as_deref(), std::env::var("XDG_SESSION_TYPE").ok().as_deref(), in_flatpak())
}

/// A Wayland session (the recording windows have no border then).
#[cfg(target_os = "linux")]
pub fn wayland_now() -> bool {
    !crate::rec_ui::x11_session(std::env::var_os("WAYLAND_DISPLAY").as_deref(), std::env::var("XDG_SESSION_TYPE").ok().as_deref())
}

#[cfg(target_os = "linux")]
pub fn in_flatpak() -> bool {
    std::env::var_os("FLATPAK_ID").is_some() || std::path::Path::new("/.flatpak-info").exists()
}

/// A token for `handle_token` / `session_handle_token`: unique in this
/// process, valid in an object path.
pub fn token() -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    format!("rustshot_{}_{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed))
}

/// The Request object a call with `token` will answer on.
pub fn request_path(unique: &str, token: &str) -> String {
    format!("{DESKTOP_PATH}/request/{}/{token}", dbus::sender_elem(unique))
}

fn response_rule(path: &str) -> String {
    dbus::signal_rule(DESKTOP, path, REQUEST, "Response")
}

/// The unique name that owns the portal (`GetNameOwner`): only its
/// Response signals are taken.
fn portal_owner(bus: &mut dyn Bus) -> Result<String> {
    let r = dbus::bus_call(bus, "GetNameOwner", vec![Value::str(DESKTOP)]).context("the desktop portal is not running")?;
    r.body.first().and_then(Value::as_str).map(String::from).ok_or_else(|| anyhow!("GetNameOwner: no name"))
}

/// Call portal method `iface.method` with `args` and an options dict (plus
/// our `handle_token`), then wait up to `timeout` for its Response; the
/// results dict. A dialog closed by the user is [`Cancelled`]; no answer in
/// time is [`NoAnswer`].
pub fn request(bus: &mut dyn Bus, iface: &str, method: &str, args: Vec<Value>, options: Vec<(&str, Value)>, timeout: Duration) -> Result<Value> {
    request_or_cancel(bus, iface, method, args, options, timeout, None)
}

/// [`request`] that `cancel` can call off while it waits (the dialog is
/// closed, the result [`Cancelled`]). Only a Response sent by the portal's
/// owner (`GetNameOwner`) counts: anyone may send signals on the bus.
pub fn request_or_cancel(
    bus: &mut dyn Bus,
    iface: &str,
    method: &str,
    mut args: Vec<Value>,
    mut options: Vec<(&str, Value)>,
    timeout: Duration,
    cancel: Option<&CancelFlag>,
) -> Result<Value> {
    if cancelled(cancel) {
        return Err(anyhow::Error::new(Cancelled));
    }
    let owner = portal_owner(bus)?;
    let tok = token();
    let path = request_path(bus.unique_name(), &tok);
    options.push(("handle_token", Value::Str(tok)));
    dbus::add_match(bus, &response_rule(&path))?;
    args.push(Value::dict_sv(options));
    let reply = bus.call(Message::call(DESKTOP, DESKTOP_PATH, iface, method, args), dbus::CALL_TIMEOUT);
    let reply = match reply {
        Ok(r) => r,
        Err(e) => {
            let _ = dbus::remove_match(bus, &response_rule(&path));
            return Err(e);
        }
    };
    let handle = reply.body.first().and_then(|v| v.as_str()).unwrap_or(&path).to_string();
    if handle != path {
        // An old portal (before 0.9) with its own path: follow it there.
        dbus::add_match(bus, &response_rule(&handle))?;
    }
    let want = |m: &Message| {
        m.is(REQUEST, "Response") && m.sender.as_deref() == Some(owner.as_str()) && m.path.as_deref().is_some_and(|p| p == path || p == handle)
    };
    // In slices, so a cancel is seen within one.
    let end = std::time::Instant::now() + timeout;
    let mut got = Ok(None);
    let mut called_off = false;
    loop {
        let left = end.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() {
            break;
        }
        let slice = if cancel.is_some() { left.min(CANCEL_POLL) } else { left };
        got = bus.wait_signal(&want, slice);
        if !matches!(got, Ok(None)) || cancel.is_none() {
            break;
        }
        if cancelled(cancel) {
            called_off = true;
            break;
        }
    }
    let _ = dbus::remove_match(bus, &response_rule(&path));
    if handle != path {
        let _ = dbus::remove_match(bus, &response_rule(&handle));
    }
    let Some(sig) = got? else {
        // Take the dialog down rather than leave it waiting for nobody.
        let _ = bus.send(close_msg(&handle, REQUEST));
        if called_off {
            return Err(anyhow::Error::new(Cancelled));
        }
        return Err(anyhow::Error::new(NoAnswer { method: method.into(), secs: timeout.as_secs() }));
    };
    let code = sig.body.first().and_then(Value::as_u32).ok_or_else(|| anyhow!("{method}: bad Response"))?;
    let results = sig.body.get(1).cloned().unwrap_or(Value::Dict("s".into(), "v".into(), Vec::new()));
    match code {
        0 => Ok(results),
        1 => Err(anyhow::Error::new(Cancelled)),
        _ => Err(anyhow::Error::new(Refused { method: method.into(), code })),
    }
}

fn close_msg(path: &str, iface: &str) -> Message {
    let mut m = Message::call(DESKTOP, path, iface, "Close", Vec::new());
    m.flags = dbus::NO_REPLY;
    m
}

/// A u32 property of a portal interface.
fn prop_u32(bus: &mut dyn Bus, iface: &str, name: &str) -> Result<u32> {
    let v = dbus::get_property(bus, DESKTOP, DESKTOP_PATH, iface, name)?;
    v.as_u32().ok_or_else(|| anyhow!("{iface}.{name} is not a number"))
}

/// One stream of a screen cast.
#[derive(Clone, Debug, PartialEq)]
pub struct StreamInfo {
    pub node: u32,
    /// Where the monitor is in the compositor's layout, when said.
    pub pos: Option<(i32, i32)>,
    /// Its size there (logical px), when said.
    pub size: Option<(i32, i32)>,
}

/// A started screen cast session.
#[derive(Clone, Debug, PartialEq)]
pub struct CastInfo {
    pub session: String,
    pub stream: StreamInfo,
    /// The token to restore this choice next time (portal version 4+).
    pub restore_token: Option<String>,
}

fn pair(v: Option<&Value>) -> Option<(i32, i32)> {
    let f = v?.items();
    Some((f.first()?.as_i32()?, f.get(1)?.as_i32()?))
}

/// Run the ScreenCast handshake up to `Start`: one monitor, the cursor in
/// the picture, the choice remembered (`restore` = the stored token).
pub fn screencast(bus: &mut dyn Bus, restore: Option<&str>) -> Result<CastInfo> {
    screencast_or_cancel(bus, restore, None)
}

/// [`screencast`] that `cancel` can call off. With a restore token `Start`
/// may take [`RESTORED`] at most (a restored choice needs no dialog).
pub fn screencast_or_cancel(bus: &mut dyn Bus, restore: Option<&str>, cancel: Option<&CancelFlag>) -> Result<CastInfo> {
    let version = prop_u32(bus, SCREENCAST, "version").context("the ScreenCast portal is not available")?;
    let cursors = prop_u32(bus, SCREENCAST, "AvailableCursorModes").unwrap_or(0);
    let sess_tok = token();
    let r = request_or_cancel(bus, SCREENCAST, "CreateSession", vec![], vec![("session_handle_token", Value::Str(sess_tok))], QUICK, cancel)?;
    let session = r.get("session_handle").and_then(Value::as_str).ok_or_else(|| anyhow!("CreateSession: no session"))?.to_string();
    let run = |bus: &mut dyn Bus| -> Result<CastInfo> {
        let mut opts = vec![("types", Value::U32(MONITOR)), ("multiple", Value::Bool(false))];
        if cursors & CURSOR_EMBEDDED != 0 {
            opts.push(("cursor_mode", Value::U32(CURSOR_EMBEDDED)));
        } else if cursors & CURSOR_HIDDEN != 0 {
            opts.push(("cursor_mode", Value::U32(CURSOR_HIDDEN)));
        }
        let mut wait = DIALOG;
        if version >= 4 {
            opts.push(("persist_mode", Value::U32(PERSIST)));
            if let Some(t) = restore.filter(|t| !t.is_empty()) {
                opts.push(("restore_token", Value::str(t)));
                wait = RESTORED;
            }
        }
        request_or_cancel(bus, SCREENCAST, "SelectSources", vec![Value::path(&session)], opts, QUICK, cancel)?;
        let r = request_or_cancel(bus, SCREENCAST, "Start", vec![Value::path(&session), Value::str("")], vec![], wait, cancel)?;
        let streams = r.get("streams").ok_or_else(|| anyhow!("Start: no streams"))?;
        let first = streams.items().first().ok_or_else(|| anyhow!("Start: no screen was shared"))?;
        let f = first.items();
        let node = f.first().and_then(Value::as_u32).ok_or_else(|| anyhow!("Start: no PipeWire node"))?;
        let props = f.get(1);
        let stream = StreamInfo {
            node,
            pos: pair(props.and_then(|p| p.get("position"))),
            size: pair(props.and_then(|p| p.get("size"))),
        };
        let restore_token = r.get("restore_token").and_then(Value::as_str).map(String::from);
        Ok(CastInfo { session: session.clone(), stream, restore_token })
    };
    let out = run(bus);
    if out.is_err() {
        let _ = bus.send(close_msg(&session, SESSION));
    }
    out
}

/// The screen cast for a recording of `area` on `mons`, with the `stored`
/// restore token: a restored choice that does not come back within
/// [`RESTORED`], is refused, or is of a monitor the area is not on, is
/// dropped (`forget`) and the dialog asked afresh.
pub fn negotiate(
    bus: &mut dyn Bus,
    stored: Option<&str>,
    area: IRect,
    mons: &[IRect],
    cancel: Option<&CancelFlag>,
    forget: &mut dyn FnMut(),
) -> Result<CastInfo> {
    let mut info = match screencast_or_cancel(bus, stored, cancel) {
        Err(e) if stored.is_some() && (e.is::<NoAnswer>() || e.is::<Refused>()) => {
            eprintln!("rustshot: the remembered screen did not come back ({e:#}); asking again");
            forget();
            return screencast_or_cancel(bus, None, cancel);
        }
        r => r?,
    };
    if stored.is_some() {
        let s = &info.stream;
        let known = s.pos.is_some() && s.size.is_some();
        if known && !touches(area, stream_rect(s, (0, 0), area, mons)) {
            eprintln!("rustshot: the remembered screen is not where the selection is; asking again");
            close_session(bus, &info.session);
            forget();
            info = screencast_or_cancel(bus, None, cancel)?;
        }
    }
    Ok(info)
}

/// `OpenPipeWireRemote` of `iface` (ScreenCast: for `session`; Camera:
/// none): the PipeWire socket.
pub fn open_remote(bus: &mut dyn Bus, iface: &str, session: Option<&str>) -> Result<Fd> {
    let mut args = Vec::new();
    if let Some(s) = session {
        args.push(Value::path(s));
    }
    args.push(Value::dict_sv(vec![]));
    let mut r = bus.call(Message::call(DESKTOP, DESKTOP_PATH, iface, "OpenPipeWireRemote", args), dbus::CALL_TIMEOUT)?;
    let v = r.body.first().cloned().ok_or_else(|| anyhow!("OpenPipeWireRemote: empty reply"))?;
    r.take_fd(&v).context("OpenPipeWireRemote")
}

/// Close a session (the stream ends).
pub fn close_session(bus: &mut dyn Bus, session: &str) {
    let _ = bus.send(close_msg(session, SESSION));
}

/// Whether the system says it has a camera (asks nobody).
pub fn camera_present(bus: &mut dyn Bus) -> Result<bool> {
    let v = dbus::get_property(bus, DESKTOP, DESKTOP_PATH, CAMERA, "IsCameraPresent")?;
    v.as_bool().ok_or_else(|| anyhow!("IsCameraPresent is not a boolean"))
}

/// Ask for the camera (a dialog the first time), then its PipeWire socket;
/// `cancel` calls the dialog off.
pub fn access_camera(bus: &mut dyn Bus, cancel: Option<&CancelFlag>) -> Result<Fd> {
    request_or_cancel(bus, CAMERA, "AccessCamera", vec![], vec![], DIALOG, cancel)?;
    open_remote(bus, CAMERA, None)
}

/// Take a screenshot of the whole desktop without the portal's dialog
/// (waiting up to `timeout`: a desktop may still ask once): the picture
/// file's URI.
pub fn screenshot(bus: &mut dyn Bus, timeout: Duration) -> Result<String> {
    let opts = vec![("interactive", Value::Bool(false)), ("modal", Value::Bool(false))];
    let r = request(bus, SCREENSHOT, "Screenshot", vec![Value::str("")], opts, timeout)?;
    r.get("uri").and_then(Value::as_str).map(String::from).ok_or_else(|| anyhow!("Screenshot: no picture in the answer"))
}

/// The local path of a `file://` URI (percent-decoded); `None` for any
/// other scheme, a remote host or a path that is not UTF-8.
pub fn file_uri_path(uri: &str) -> Option<String> {
    let rest = uri.strip_prefix("file://")?;
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    if !rest.starts_with('/') {
        return None;
    }
    let b = rest.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let hex = std::str::from_utf8(b.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// `rect` (X screen coordinates, the screen being `screen` px) out of a
/// screenshot `img` of that screen, which may be at another scale (a
/// HiDPI output): nearest-neighbour to exactly `rect`'s size, the part
/// outside the picture black. `None` for an empty rect or picture.
pub fn crop_screen(img: &crate::pixbuf::PixBuf, screen: (u32, u32), rect: IRect) -> Option<crate::pixbuf::PixBuf> {
    let (iw, ih) = img.dimensions();
    let (sw, sh) = if screen.0 == 0 || screen.1 == 0 { (iw, ih) } else { screen };
    if rect.2 == 0 || rect.3 == 0 || iw == 0 || ih == 0 {
        return None;
    }
    if (sw, sh) == (iw, ih) && rect.0 >= 0 && rect.1 >= 0 && rect.0 as u32 + rect.2 <= iw && rect.1 as u32 + rect.3 <= ih {
        return Some(img.crop(rect.0 as u32, rect.1 as u32, rect.2, rect.3));
    }
    let (kx, ky) = (iw as f64 / sw as f64, ih as f64 / sh as f64);
    let src = img.as_raw();
    let mut out = vec![0u8; rect.2 as usize * rect.3 as usize * 4];
    for (y, row) in out.chunks_exact_mut(rect.2 as usize * 4).enumerate() {
        let sy = ((rect.1 as f64 + y as f64 + 0.5) * ky).floor() as i64;
        for (x, px) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let sx = ((rect.0 as f64 + x as f64 + 0.5) * kx).floor() as i64;
            *px = if sx >= 0 && sy >= 0 && (sx as u32) < iw && (sy as u32) < ih {
                let i = (sy as usize * iw as usize + sx as usize) * 4;
                [src[i], src[i + 1], src[i + 2], 255]
            } else {
                [0, 0, 0, 255]
            };
        }
    }
    crate::pixbuf::PixBuf::from_raw(rect.2, rect.3, out)
}

// ---------------------------------------------------------------------------
// Where the shared monitor is, and the crop
// ---------------------------------------------------------------------------

fn contains(r: IRect, x: i32, y: i32) -> bool {
    x >= r.0 && y >= r.1 && x < r.0 + r.2 as i32 && y < r.1 + r.3 as i32
}

/// Whether `a` and `b` (w, h) have the same aspect, within 1 %.
fn same_aspect(a: (u32, u32), b: (u32, u32)) -> bool {
    let (l, r) = (a.0 as f64 * b.1 as f64, a.1 as f64 * b.0 as f64);
    l > 0.0 && r > 0.0 && (l - r).abs() <= 0.01 * l.max(r)
}

/// Where the shared monitor is in X screen coordinates (those of `area`
/// and of the XRandR monitors `mons`), so the selection can be cut from
/// its `frame`-sized pictures; `None` when that cannot be told (the whole
/// shared screen is recorded and the user is told).
///
/// The portal gives the monitor's position and size in the compositor's
/// logical layout. X clients (XWayland) see either that same layout
/// (GNOME; KDE scaling X clients itself off) or one scaled by the X scale
/// (KDE with "X applications scale themselves": X px = logical × scale). So
/// the XRandR monitor whose place is the portal's times some scale `k`
/// (k = 1 included) is the one; with a single monitor, that one. Its size
/// must agree with the portal's (and the pictures') aspect, else `None`.
/// Without any XRandR monitors the portal's rect is taken as it is.
/// Without a position: the monitor of the portal's size (or the
/// picture's), the one with the area's centre first; else the only one.
pub fn stream_rect(s: &StreamInfo, frame: (u32, u32), area: IRect, mons: &[IRect]) -> Option<IRect> {
    let size = s.size.filter(|z| z.0 > 0 && z.1 > 0).map(|z| (z.0 as u32, z.1 as u32));
    let pictures_agree = |m: &IRect| frame.0 == 0 || frame.1 == 0 || same_aspect((m.2, m.3), frame);
    if let (Some(p), Some(z)) = (s.pos, size) {
        if mons.is_empty() {
            return Some((p.0, p.1, z.0, z.1));
        }
        let near = |a: i32, b: f64| (a as f64 - b).abs() <= 2.0;
        let placed = mons.iter().find(|m| {
            let k = m.2 as f64 / z.0 as f64;
            near(m.3 as i32, z.1 as f64 * k) && near(m.0, p.0 as f64 * k) && near(m.1, p.1 as f64 * k)
        });
        let m = placed.or_else(|| (mons.len() == 1).then(|| &mons[0]))?;
        return (same_aspect((m.2, m.3), z) && pictures_agree(m)).then_some(*m);
    }
    let (cx, cy) = (area.0 + area.2 as i32 / 2, area.1 + area.3 as i32 / 2);
    let fits = |m: &&IRect| size.is_some_and(|z| (m.2, m.3) == z) || (m.2, m.3) == frame;
    let mut cands: Vec<&IRect> = mons.iter().filter(fits).collect();
    cands.sort_by_key(|m| !contains(**m, cx, cy));
    if let Some(m) = cands.first() {
        return Some(**m);
    }
    (mons.len() == 1).then(|| mons[0])
}

/// The part of a `frame`-sized picture of `screen` (X screen coordinates)
/// that shows `area`, in picture pixels; and whether the area lies within
/// the screen (else the whole picture, and the user is told).
pub fn crop_for(area: IRect, screen: Option<IRect>, frame: (u32, u32)) -> (IRect, bool) {
    let whole = (0, 0, frame.0.max(1), frame.1.max(1));
    let Some(s) = screen.filter(|s| s.2 > 0 && s.3 > 0) else { return (whole, false) };
    let inside = area.0 >= s.0 && area.1 >= s.1 && area.0 + area.2 as i32 <= s.0 + s.2 as i32 && area.1 + area.3 as i32 <= s.1 + s.3 as i32;
    if !inside || area.2 == 0 || area.3 == 0 {
        return (whole, false);
    }
    let (kx, ky) = (frame.0 as f64 / s.2 as f64, frame.1 as f64 / s.3 as f64);
    let x0 = ((area.0 - s.0) as f64 * kx).floor().max(0.0) as u32;
    let y0 = ((area.1 - s.1) as f64 * ky).floor().max(0.0) as u32;
    let x1 = (((area.0 - s.0) as f64 + area.2 as f64) * kx).ceil().min(frame.0 as f64) as u32;
    let y1 = (((area.1 - s.1) as f64 + area.3 as f64) * ky).ceil().min(frame.1 as f64) as u32;
    if x1 <= x0 || y1 <= y0 {
        return (whole, false);
    }
    ((x0 as i32, y0 as i32, x1 - x0, y1 - y0), true)
}

/// What is said when the area could not be cropped from the shared screen.
pub const WHOLE_SCREEN: &str =
    "the selection is not within the screen shared in the portal dialog; the whole shared screen was recorded";

/// Whether `area` lies on the shared monitor `screen` at all (a restored
/// choice of another monitor is asked again).
pub fn touches(area: IRect, screen: Option<IRect>) -> bool {
    let Some(s) = screen else { return true };
    area.0 < s.0 + s.2 as i32 && s.0 < area.0 + area.2 as i32 && area.1 < s.1 + s.3 as i32 && s.1 < area.1 + area.3 as i32
}

// ---------------------------------------------------------------------------
// The restore token
// ---------------------------------------------------------------------------

/// `$XDG_STATE_HOME/rustshot/screencast-token`, else
/// `~/.local/state/rustshot/screencast-token`.
pub fn token_path(state_home: Option<&str>, home: Option<&str>) -> Option<PathBuf> {
    let base = match state_home.filter(|s| s.starts_with('/')) {
        Some(s) => PathBuf::from(s),
        None => PathBuf::from(home.filter(|h| !h.is_empty())?).join(".local/state"),
    };
    Some(base.join("rustshot").join("screencast-token"))
}

#[cfg(target_os = "linux")]
fn token_file() -> Option<PathBuf> {
    token_path(std::env::var("XDG_STATE_HOME").ok().as_deref(), std::env::var("HOME").ok().as_deref())
}

/// The stored restore token.
#[cfg(target_os = "linux")]
pub fn load_token() -> Option<String> {
    let t = std::fs::read_to_string(token_file()?).ok()?;
    let t = t.trim();
    (!t.is_empty() && t.len() < 4096).then(|| t.to_string())
}

/// Store (or with `None` remove) the restore token, readable by us only.
#[cfg(target_os = "linux")]
pub fn save_token(t: Option<&str>) {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let Some(p) = token_file() else { return };
    let Some(t) = t else {
        let _ = std::fs::remove_file(&p);
        return;
    };
    let r = (|| -> std::io::Result<()> {
        let dir = p.parent().expect("has a parent");
        std::fs::create_dir_all(dir)?;
        let tmp = p.with_extension("tmp");
        let _ = std::fs::remove_file(&tmp);
        let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&tmp)?;
        f.write_all(t.as_bytes())?;
        f.sync_all()?;
        std::fs::rename(&tmp, &p)
    })();
    if let Err(e) = r {
        eprintln!("rustshot: cannot store the screen cast token in {}: {e}", p.display());
    }
}

// ---------------------------------------------------------------------------
// Live (Linux)
// ---------------------------------------------------------------------------

/// How long the ScreenCast probe may take (connecting, authenticating and
/// reading the portal's `version`), all told.
#[cfg(target_os = "linux")]
pub const PROBE: Duration = Duration::from_secs(2);

#[cfg(target_os = "linux")]
static PROBED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

/// Whether the ScreenCast portal answers, without waiting: `None` while it
/// is being asked (the first call starts the probe on its own thread; it
/// takes [`PROBE`] at most), then the answer, cached.
#[cfg(target_os = "linux")]
pub fn screencast_status() -> Option<bool> {
    static START: std::sync::Once = std::sync::Once::new();
    if let Some(v) = PROBED.get() {
        return Some(*v);
    }
    START.call_once(|| {
        let spawned = std::thread::Builder::new().name("portal-probe".into()).spawn(|| {
            let _ = PROBED.set(probe_screencast());
        });
        if spawned.is_err() {
            let _ = PROBED.set(probe_screencast());
        }
    });
    PROBED.get().copied()
}

/// [`screencast_status`], waiting for the answer (tests, the command line).
#[cfg(target_os = "linux")]
pub fn screencast_available() -> bool {
    let end = std::time::Instant::now() + PROBE + Duration::from_millis(500);
    loop {
        if let Some(v) = screencast_status() {
            return v;
        }
        if std::time::Instant::now() >= end {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[cfg(target_os = "linux")]
fn probe_screencast() -> bool {
    let r = (|| -> Result<u32> {
        let end = std::time::Instant::now() + PROBE;
        let mut c = dbus::Conn::session_within(PROBE)?;
        let left = end.saturating_duration_since(std::time::Instant::now());
        let m = Message::call(DESKTOP, DESKTOP_PATH, "org.freedesktop.DBus.Properties", "Get", vec![Value::str(SCREENCAST), Value::str("version")]);
        let r = c.call(m, left.max(Duration::from_millis(1)))?;
        r.body.first().and_then(Value::as_u32).ok_or_else(|| anyhow!("ScreenCast.version is not a number"))
    })();
    match r {
        Ok(v) => {
            eprintln!("rustshot: ScreenCast portal version {v}");
            true
        }
        Err(e) => {
            eprintln!("rustshot: no ScreenCast portal ({e:#}); recording is off");
            false
        }
    }
}

/// A screenshot of the whole desktop through the Screenshot portal (no
/// dialog), waiting up to `timeout` for it. The portal's file is removed
/// once read: it is ours, made only for this capture (some desktops put it
/// in ~/Pictures).
#[cfg(target_os = "linux")]
pub fn grab_screen(timeout: Duration) -> Result<crate::pixbuf::PixBuf> {
    let mut conn = dbus::Conn::session_within(timeout.min(dbus::CALL_TIMEOUT)).context("connect to the session bus")?;
    let uri = screenshot(&mut conn, timeout)?;
    let path = file_uri_path(&uri).ok_or_else(|| anyhow!("Screenshot: not a local file: {uri}"))?;
    let bytes = std::fs::read(&path).with_context(|| format!("read the screenshot {path}"))?;
    let _ = std::fs::remove_file(&path);
    crate::pixbuf::PixBuf::from_png(&bytes).with_context(|| format!("decode the screenshot {path}"))
}

/// What a recording ends with when the screen share stops under it (the
/// desktop's "stop sharing", the portal closing the session).
pub const SHARING_STOPPED: &str = "Screen sharing was stopped";

/// A running screen cast: the bus connection that holds the session, the
/// PipeWire socket (taken by the frame source) and the stream.
#[cfg(target_os = "linux")]
pub struct Cast {
    conn: dbus::Conn,
    /// The portal's unique name (its `Closed` signal comes from it).
    owner: String,
    pub info: CastInfo,
    pub fd: Option<Fd>,
}

#[cfg(target_os = "linux")]
impl Cast {
    /// Ask for a screen (the portal's dialog unless a stored choice is
    /// restored) for a recording of `area` on `mons`. A restored choice of
    /// a monitor that `area` is not on, or one that does not come back
    /// within [`RESTORED`] (or is refused), is asked again. Blocks while
    /// the dialog is open.
    #[allow(dead_code)] // tests; the app calls it off with `start_or_cancel`
    pub fn start(area: IRect, mons: &[IRect]) -> Result<Cast> {
        Cast::start_or_cancel(area, mons, None)
    }

    /// [`Cast::start`] that `cancel` calls off ([`Cancelled`], the dialog
    /// closed), for a worker thread while the UI stays live.
    pub fn start_or_cancel(area: IRect, mons: &[IRect], cancel: Option<&CancelFlag>) -> Result<Cast> {
        let mut conn = dbus::Conn::session().context("connect to the session bus")?;
        let owner = portal_owner(&mut conn)?;
        let info = negotiate(&mut conn, load_token().as_deref(), area, mons, cancel, &mut || save_token(None))?;
        save_token(info.restore_token.as_deref());
        // From here on dropping the cast closes the session.
        let mut cast = Cast { conn, owner, info, fd: None };
        let rule = dbus::signal_rule(DESKTOP, &cast.info.session, SESSION, "Closed");
        if let Err(e) = dbus::add_match(&mut cast.conn, &rule) {
            eprintln!("rustshot: the screen cast session's end will not be noticed: {e:#}");
        }
        cast.fd = Some(open_remote(&mut cast.conn, SCREENCAST, Some(&cast.info.session))?);
        let i = &cast.info;
        eprintln!("rustshot: screen cast node {} at {:?} size {:?}", i.stream.node, i.stream.pos, i.stream.size);
        Ok(cast)
    }

    /// Whether the portal closed the session (looked at without waiting),
    /// or the bus connection that holds it broke.
    pub fn closed(&mut self) -> bool {
        let (session, owner) = (self.info.session.clone(), self.owner.clone());
        let want = |m: &Message| m.is(SESSION, "Closed") && m.path.as_deref() == Some(&session) && m.sender.as_deref() == Some(&owner);
        match self.conn.wait_signal(&want, Duration::ZERO) {
            Ok(Some(_)) => {
                eprintln!("rustshot: the portal closed the screen cast session");
                true
            }
            Ok(None) => false,
            Err(e) => {
                eprintln!("rustshot: screen cast session bus: {e:#}");
                self.conn.broken().is_some()
            }
        }
    }

    /// Close the session now (tests: the share stopping under a recording).
    #[allow(dead_code)]
    pub fn close_now(&mut self) {
        close_session(&mut self.conn, &self.info.session);
    }
}

#[cfg(target_os = "linux")]
impl Drop for Cast {
    fn drop(&mut self) {
        close_session(&mut self.conn, &self.info.session);
    }
}

#[cfg(test)]
mod tests;
