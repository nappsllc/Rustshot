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
use anyhow::{anyhow, bail, Context, Result};
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

/// Call portal method `iface.method` with `args` and an options dict (plus
/// our `handle_token`), then wait up to `timeout` for its Response; the
/// results dict. A dialog closed by the user is [`Cancelled`].
pub fn request(bus: &mut dyn Bus, iface: &str, method: &str, mut args: Vec<Value>, mut options: Vec<(&str, Value)>, timeout: Duration) -> Result<Value> {
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
    let got = bus.wait_signal(
        &|m| m.is(REQUEST, "Response") && m.path.as_deref().is_some_and(|p| p == path || p == handle),
        timeout,
    );
    let _ = dbus::remove_match(bus, &response_rule(&path));
    if handle != path {
        let _ = dbus::remove_match(bus, &response_rule(&handle));
    }
    let Some(sig) = got? else {
        // Take the dialog down rather than leave it waiting for nobody.
        let _ = bus.send(close_msg(&handle, REQUEST));
        bail!("{method}: the portal did not answer within {} s", timeout.as_secs());
    };
    let code = sig.body.first().and_then(Value::as_u32).ok_or_else(|| anyhow!("{method}: bad Response"))?;
    let results = sig.body.get(1).cloned().unwrap_or(Value::Dict("s".into(), "v".into(), Vec::new()));
    match code {
        0 => Ok(results),
        1 => Err(anyhow::Error::new(Cancelled)),
        _ => bail!("{method}: the portal refused (response {code})"),
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
    let version = prop_u32(bus, SCREENCAST, "version").context("the ScreenCast portal is not available")?;
    let cursors = prop_u32(bus, SCREENCAST, "AvailableCursorModes").unwrap_or(0);
    let sess_tok = token();
    let r = request(bus, SCREENCAST, "CreateSession", vec![], vec![("session_handle_token", Value::Str(sess_tok))], QUICK)?;
    let session = r.get("session_handle").and_then(Value::as_str).ok_or_else(|| anyhow!("CreateSession: no session"))?.to_string();
    let run = |bus: &mut dyn Bus| -> Result<CastInfo> {
        let mut opts = vec![("types", Value::U32(MONITOR)), ("multiple", Value::Bool(false))];
        if cursors & CURSOR_EMBEDDED != 0 {
            opts.push(("cursor_mode", Value::U32(CURSOR_EMBEDDED)));
        } else if cursors & CURSOR_HIDDEN != 0 {
            opts.push(("cursor_mode", Value::U32(CURSOR_HIDDEN)));
        }
        if version >= 4 {
            opts.push(("persist_mode", Value::U32(PERSIST)));
            if let Some(t) = restore.filter(|t| !t.is_empty()) {
                opts.push(("restore_token", Value::str(t)));
            }
        }
        request(bus, SCREENCAST, "SelectSources", vec![Value::path(&session)], opts, QUICK)?;
        let r = request(bus, SCREENCAST, "Start", vec![Value::path(&session), Value::str("")], vec![], DIALOG)?;
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

/// Ask for the camera (a dialog the first time), then its PipeWire socket.
pub fn access_camera(bus: &mut dyn Bus) -> Result<Fd> {
    request(bus, CAMERA, "AccessCamera", vec![], vec![], DIALOG)?;
    open_remote(bus, CAMERA, None)
}

/// Take a screenshot of the whole desktop without the portal's dialog:
/// the picture file's URI.
pub fn screenshot(bus: &mut dyn Bus) -> Result<String> {
    let opts = vec![("interactive", Value::Bool(false)), ("modal", Value::Bool(false))];
    let r = request(bus, SCREENSHOT, "Screenshot", vec![Value::str("")], opts, DIALOG)?;
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

/// Where the shared monitor is on the X screen: the portal's position and
/// size when it gives both; else the monitor of `mons` that has the
/// portal's size (or the picture's, `frame`), the one with the area's
/// centre first; else the monitor with the area's centre when there is
/// only one monitor. `None`: not known (the whole picture is recorded).
pub fn stream_rect(s: &StreamInfo, frame: (u32, u32), area: IRect, mons: &[IRect]) -> Option<IRect> {
    if let (Some(p), Some(z)) = (s.pos, s.size)
        && z.0 > 0
        && z.1 > 0
    {
        return Some((p.0, p.1, z.0 as u32, z.1 as u32));
    }
    let (cx, cy) = (area.0 + area.2 as i32 / 2, area.1 + area.3 as i32 / 2);
    let size = s.size.filter(|z| z.0 > 0 && z.1 > 0).map(|z| (z.0 as u32, z.1 as u32));
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

/// Whether the ScreenCast portal answers (cached; asked once, 2 s at most).
#[cfg(target_os = "linux")]
pub fn screencast_available() -> bool {
    static OK: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *OK.get_or_init(|| {
        let r = (|| -> Result<u32> {
            let mut c = dbus::Conn::session()?;
            prop_u32(&mut c, SCREENCAST, "version")
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
    })
}

/// A screenshot of the whole desktop through the Screenshot portal (no
/// dialog). The portal's file is removed once read: it is ours, made only
/// for this capture (some desktops put it in ~/Pictures).
#[cfg(target_os = "linux")]
pub fn grab_screen() -> Result<crate::pixbuf::PixBuf> {
    let mut conn = dbus::Conn::session().context("connect to the session bus")?;
    let uri = screenshot(&mut conn)?;
    let path = file_uri_path(&uri).ok_or_else(|| anyhow!("Screenshot: not a local file: {uri}"))?;
    let bytes = std::fs::read(&path).with_context(|| format!("read the screenshot {path}"))?;
    let _ = std::fs::remove_file(&path);
    crate::pixbuf::PixBuf::from_png(&bytes).with_context(|| format!("decode the screenshot {path}"))
}

/// A running screen cast: the bus connection that holds the session, the
/// PipeWire socket (taken by the frame source) and the stream.
#[cfg(target_os = "linux")]
pub struct Cast {
    conn: dbus::Conn,
    pub info: CastInfo,
    pub fd: Option<Fd>,
}

#[cfg(target_os = "linux")]
impl Cast {
    /// Ask for a screen (the portal's dialog unless a stored choice is
    /// restored) for a recording of `area` on `mons`. A restored choice of
    /// a monitor that `area` is not on is asked again. Blocks while the
    /// dialog is open.
    pub fn start(area: IRect, mons: &[IRect]) -> Result<Cast> {
        let mut conn = dbus::Conn::session().context("connect to the session bus")?;
        let stored = load_token();
        let mut info = screencast(&mut conn, stored.as_deref())?;
        if stored.is_some() {
            let s = &info.stream;
            let known = s.pos.is_some() && s.size.is_some();
            if known && !touches(area, stream_rect(s, (0, 0), area, mons)) {
                eprintln!("rustshot: the remembered screen is not where the selection is; asking again");
                close_session(&mut conn, &info.session);
                save_token(None);
                info = screencast(&mut conn, None)?;
            }
        }
        save_token(info.restore_token.as_deref());
        let fd = open_remote(&mut conn, SCREENCAST, Some(&info.session))?;
        eprintln!("rustshot: screen cast node {} at {:?} size {:?}", info.stream.node, info.stream.pos, info.stream.size);
        Ok(Cast { conn, info, fd: Some(fd) })
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
