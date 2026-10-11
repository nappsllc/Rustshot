use super::*;
use crate::dbus::{Kind, Message, Value};
use std::collections::VecDeque;

/// A portal method's answer: reply body, then an optional Response.
type Answer = Result<(Vec<Value>, Option<(u32, Value)>)>;

/// A scripted bus: answers AddMatch/RemoveMatch and property reads, and
/// portal methods through `answer` (reply body, then an optional Response
/// signal: code and results, sent on the request path the call's
/// `handle_token` implies, or on `path_override`).
struct Fake {
    name: String,
    props: Vec<(&'static str, &'static str, Value)>,
    answer: Box<dyn FnMut(&Message) -> Answer>,
    /// Answer on another path than the computed one (an old portal).
    path_override: Option<String>,
    /// Signals not yet taken.
    signals: VecDeque<Message>,
    /// Every call made (member names), and the match rules in force.
    calls: Vec<Message>,
    rules: Vec<String>,
    sent: Vec<Message>,
}

impl Fake {
    fn new(answer: impl FnMut(&Message) -> Answer + 'static) -> Fake {
        Fake {
            name: ":1.42".into(),
            props: vec![(SCREENCAST, "version", Value::U32(5)), (SCREENCAST, "AvailableCursorModes", Value::U32(7))],
            answer: Box::new(answer),
            path_override: None,
            signals: VecDeque::new(),
            calls: Vec::new(),
            rules: Vec::new(),
            sent: Vec::new(),
        }
    }

    fn members(&self) -> Vec<String> {
        self.calls.iter().filter_map(|m| m.member.clone()).collect()
    }
}

fn opts_token(m: &Message) -> String {
    m.body.last().and_then(|d| d.get("handle_token")).and_then(Value::as_str).expect("handle_token").to_string()
}

impl Bus for Fake {
    fn unique_name(&self) -> &str {
        &self.name
    }

    fn call(&mut self, m: Message, _timeout: Duration) -> Result<Message> {
        let mut call = m;
        call.serial = self.calls.len() as u32 + 1;
        let member = call.member.clone().unwrap_or_default();
        let reply = match member.as_str() {
            "AddMatch" => {
                self.rules.push(call.body[0].as_str().unwrap().into());
                Message::reply(&call, vec![])
            }
            "RemoveMatch" => {
                let r = call.body[0].as_str().unwrap().to_string();
                self.rules.retain(|x| *x != r);
                Message::reply(&call, vec![])
            }
            "Get" => {
                let (i, p) = (call.body[0].as_str().unwrap(), call.body[1].as_str().unwrap());
                match self.props.iter().find(|x| x.0 == i && x.1 == p) {
                    Some(x) => Message::reply(&call, vec![Value::variant(x.2.clone())]),
                    None => {
                        self.calls.push(call);
                        bail!("org.freedesktop.DBus.Error.UnknownInterface");
                    }
                }
            }
            _ => {
                let (body, resp) = (self.answer)(&call)?;
                if let Some((code, results)) = resp {
                    let path = self.path_override.clone().unwrap_or_else(|| request_path(&self.name, &opts_token(&call)));
                    // Only delivered when subscribed before the call (a fast portal).
                    if self.rules.iter().any(|r| r.contains(&format!("path='{path}'"))) || self.path_override.is_some() {
                        let mut s = Message::signal(&path, REQUEST, "Response", vec![Value::U32(code), results]);
                        s.sender = Some(":1.5".into());
                        self.signals.push_back(s);
                    }
                }
                let mut r = Message::reply(&call, body);
                if member == "OpenPipeWireRemote" {
                    r.fds.push(crate::dbus::tests::dummy_fd());
                }
                r
            }
        };
        self.calls.push(call);
        assert_eq!(reply.kind, Kind::Return);
        Ok(reply)
    }

    fn send(&mut self, m: Message) -> Result<u32> {
        self.sent.push(m);
        Ok(0)
    }

    fn wait_signal(&mut self, want: &dyn Fn(&Message) -> bool, _timeout: Duration) -> Result<Option<Message>> {
        let i = self.signals.iter().position(want);
        Ok(i.and_then(|i| self.signals.remove(i)))
    }
}

fn results(items: Vec<(&str, Value)>) -> Value {
    Value::dict_sv(items)
}

/// A portal that does what it is asked.
fn happy(m: &Message) -> Answer {
    let handle = || Value::path(&request_path(":1.42", &opts_token(m)));
    Ok(match m.member.as_deref().unwrap() {
        "CreateSession" => (vec![handle()], Some((0, results(vec![("session_handle", Value::str("/org/freedesktop/portal/desktop/session/1_42/s"))])))),
        "SelectSources" => (vec![handle()], Some((0, results(vec![])))),
        "Start" => {
            let props = Value::dict_sv(vec![
                ("position", Value::Struct(vec![Value::I32(1920), Value::I32(0)])),
                ("size", Value::Struct(vec![Value::I32(1280), Value::I32(720)])),
                ("source_type", Value::U32(1)),
            ]);
            let streams = Value::Array("(ua{sv})".into(), vec![Value::Struct(vec![Value::U32(57), props])]);
            (vec![handle()], Some((0, results(vec![("streams", streams), ("restore_token", Value::str("tok-2"))]))))
        }
        "OpenPipeWireRemote" => (vec![Value::Fd(0)], None),
        other => panic!("unexpected {other}"),
    })
}

#[test]
fn request_paths_come_from_the_unique_name_and_token() {
    assert_eq!(request_path(":1.42", "t_1"), "/org/freedesktop/portal/desktop/request/1_42/t_1");
    let t = token();
    assert!(t.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_'), "{t}");
    assert_ne!(token(), token());
}

#[test]
fn screencast_runs_create_select_start() {
    let mut bus = Fake::new(happy);
    let info = screencast(&mut bus, Some("tok-1")).unwrap();
    assert_eq!(info.session, "/org/freedesktop/portal/desktop/session/1_42/s");
    assert_eq!(info.stream, StreamInfo { node: 57, pos: Some((1920, 0)), size: Some((1280, 720)) });
    assert_eq!(info.restore_token.as_deref(), Some("tok-2"));
    let portal: Vec<String> = bus.members().into_iter().filter(|m| !m.ends_with("Match") && m != "Get").collect();
    assert_eq!(portal, ["CreateSession", "SelectSources", "Start"]);
    // SelectSources: a monitor, the cursor embedded, persisted with the token.
    let sel = bus.calls.iter().find(|m| m.member.as_deref() == Some("SelectSources")).unwrap();
    assert_eq!(sel.body[0], Value::path(&info.session));
    let o = &sel.body[1];
    assert_eq!(o.get("types").and_then(Value::as_u32), Some(1));
    assert_eq!(o.get("cursor_mode").and_then(Value::as_u32), Some(2));
    assert_eq!(o.get("persist_mode").and_then(Value::as_u32), Some(2));
    assert_eq!(o.get("restore_token").and_then(Value::as_str), Some("tok-1"));
    assert_eq!(o.get("multiple").and_then(Value::as_bool), Some(false));
    // CreateSession named its session token.
    let cs = &bus.calls.iter().find(|m| m.member.as_deref() == Some("CreateSession")).unwrap().body[0];
    assert!(cs.get("session_handle_token").is_some());
    // Every match rule was removed again.
    assert!(bus.rules.is_empty(), "{:?}", bus.rules);
    // The descriptor comes with OpenPipeWireRemote.
    assert!(open_remote(&mut bus, SCREENCAST, Some(&info.session)).is_ok());
    let opw = bus.calls.last().unwrap();
    assert_eq!(opw.body[0], Value::path(&info.session));
}

#[test]
fn old_portals_without_persistence_get_no_token() {
    let mut bus = Fake::new(happy);
    bus.props = vec![(SCREENCAST, "version", Value::U32(3)), (SCREENCAST, "AvailableCursorModes", Value::U32(1))];
    screencast(&mut bus, Some("tok-1")).unwrap();
    let sel = bus.calls.iter().find(|m| m.member.as_deref() == Some("SelectSources")).unwrap();
    assert!(sel.body[1].get("persist_mode").is_none());
    assert!(sel.body[1].get("restore_token").is_none());
    assert_eq!(sel.body[1].get("cursor_mode").and_then(Value::as_u32), Some(1), "hidden when embedded is not offered");
}

#[test]
fn a_cancelled_dialog_is_cancelled_and_the_session_closed() {
    let mut bus = Fake::new(|m: &Message| {
        if m.member.as_deref() == Some("Start") {
            let handle = Value::path(&request_path(":1.42", &opts_token(m)));
            return Ok((vec![handle], Some((1, results(vec![])))));
        }
        happy(m)
    });
    let e = screencast(&mut bus, None).unwrap_err();
    assert!(e.is::<Cancelled>(), "{e:#}");
    let close = bus.sent.iter().find(|m| m.member.as_deref() == Some("Close")).expect("session closed");
    assert_eq!(close.path.as_deref(), Some("/org/freedesktop/portal/desktop/session/1_42/s"));
    assert!(bus.rules.is_empty());
}

#[test]
fn a_refusal_is_an_error() {
    let mut bus = Fake::new(|m: &Message| {
        if m.member.as_deref() == Some("SelectSources") {
            let handle = Value::path(&request_path(":1.42", &opts_token(m)));
            return Ok((vec![handle], Some((2, results(vec![])))));
        }
        happy(m)
    });
    let e = screencast(&mut bus, None).unwrap_err();
    assert!(!e.is::<Cancelled>());
    assert!(format!("{e:#}").contains("refused"), "{e:#}");
}

#[test]
fn an_unanswered_request_times_out_and_closes_its_dialog() {
    let mut bus = Fake::new(|m: &Message| {
        if m.member.as_deref() == Some("Start") {
            return Ok((vec![Value::path(&request_path(":1.42", &opts_token(m)))], None));
        }
        happy(m)
    });
    let e = screencast(&mut bus, None).unwrap_err();
    assert!(format!("{e:#}").contains("did not answer"), "{e:#}");
    assert!(bus.sent.iter().any(|m| m.iface.as_deref() == Some("org.freedesktop.portal.Request")));
}

#[test]
fn an_old_portal_answering_on_its_own_path_is_followed() {
    let mut bus = Fake::new(|m: &Message| {
        let mut r = happy(m)?;
        if m.member.as_deref() != Some("OpenPipeWireRemote") {
            r.0 = vec![Value::path("/org/freedesktop/portal/desktop/request/legacy")];
        }
        Ok(r)
    });
    bus.path_override = Some("/org/freedesktop/portal/desktop/request/legacy".into());
    let info = screencast(&mut bus, None).unwrap();
    assert_eq!(info.stream.node, 57);
}

#[test]
fn a_missing_portal_says_so() {
    let mut bus = Fake::new(happy);
    bus.props.clear();
    let e = screencast(&mut bus, None).unwrap_err();
    assert!(format!("{e:#}").contains("ScreenCast portal is not available"), "{e:#}");
}

#[test]
fn the_camera_portal_asks_then_opens_the_remote() {
    let mut bus = Fake::new(|m: &Message| match m.member.as_deref().unwrap() {
        "AccessCamera" => Ok((vec![Value::path(&request_path(":1.42", &opts_token(m)))], Some((0, results(vec![]))))),
        "OpenPipeWireRemote" => Ok((vec![Value::Fd(0)], None)),
        o => panic!("unexpected {o}"),
    });
    bus.props.push((CAMERA, "IsCameraPresent", Value::Bool(true)));
    assert!(camera_present(&mut bus).unwrap());
    assert!(access_camera(&mut bus).is_ok());
    let opw = bus.calls.iter().find(|m| m.member.as_deref() == Some("OpenPipeWireRemote")).unwrap();
    assert_eq!(opw.iface.as_deref(), Some(CAMERA));
    assert_eq!(opw.body.len(), 1, "options only");
}

#[test]
fn portals_are_needed_on_wayland_and_in_flatpak() {
    use std::ffi::OsStr;
    assert!(!needs_portal(None, Some("x11"), false));
    assert!(needs_portal(Some(OsStr::new("wayland-0")), Some("wayland"), false));
    assert!(needs_portal(Some(OsStr::new("wayland-0")), None, false), "XWayland");
    assert!(needs_portal(None, Some("x11"), true), "Flatpak");
}

#[test]
fn the_shared_monitor_is_found() {
    let mons = [(0, 0, 1920, 1080), (1920, 0, 1280, 720)];
    let area = (2000, 100, 400, 300);
    let given = StreamInfo { node: 1, pos: Some((1920, 0)), size: Some((1280, 720)) };
    assert_eq!(stream_rect(&given, (2560, 1440), area, &mons), Some((1920, 0, 1280, 720)));
    // No position: the monitor of the portal's size.
    let sized = StreamInfo { node: 1, pos: None, size: Some((1280, 720)) };
    assert_eq!(stream_rect(&sized, (2560, 1440), area, &mons), Some((1920, 0, 1280, 720)));
    // Nothing said: the monitor of the picture's size; else the only one.
    let bare = StreamInfo { node: 1, pos: None, size: None };
    assert_eq!(stream_rect(&bare, (1920, 1080), (10, 10, 5, 5), &mons), Some((0, 0, 1920, 1080)));
    assert_eq!(stream_rect(&bare, (999, 999), area, &mons), None);
    assert_eq!(stream_rect(&bare, (999, 999), area, &mons[..1]), Some((0, 0, 1920, 1080)));
}

#[test]
fn the_area_is_cropped_from_the_shared_screen() {
    // A 1280x720 logical monitor at 1920,0 delivered at 2x.
    let s = Some((1920, 0, 1280, 720));
    assert_eq!(crop_for((2000, 100, 400, 300), s, (2560, 1440)), ((160, 200, 800, 600), true));
    // The whole monitor.
    assert_eq!(crop_for((1920, 0, 1280, 720), s, (2560, 1440)), ((0, 0, 2560, 1440), true));
    // Spilling over: the whole picture, and the user is told.
    assert_eq!(crop_for((1900, 0, 400, 300), s, (2560, 1440)), ((0, 0, 2560, 1440), false));
    // Unknown screen: the whole picture.
    assert_eq!(crop_for((0, 0, 10, 10), None, (640, 480)), ((0, 0, 640, 480), false));
    // Fractional scale rounds outward.
    let (c, ok) = crop_for((1921, 1, 3, 3), s, (1600, 900));
    assert!(ok);
    assert_eq!(c, (1, 1, 4, 4));
    assert!(touches((2000, 0, 10, 10), s));
    assert!(!touches((0, 0, 100, 100), s));
    assert!(touches((0, 0, 1, 1), None));
}

#[test]
fn the_token_lives_in_the_state_dir() {
    let p = |b: &str| Some(std::path::PathBuf::from(b).join("rustshot").join("screencast-token"));
    assert_eq!(token_path(Some("/st"), Some("/home/u")), p("/st"));
    assert_eq!(token_path(None, Some("/home/u")), p("/home/u/.local/state"));
    assert_eq!(token_path(Some("relative"), Some("/home/u")), p("/home/u/.local/state"));
    assert_eq!(token_path(None, None), None);
}

#[test]
fn the_screenshot_portal_answers_with_a_file() {
    let mut bus = Fake::new(|m| {
        assert_eq!(m.iface.as_deref(), Some(SCREENSHOT));
        assert_eq!(m.member.as_deref(), Some("Screenshot"));
        assert_eq!(m.body[0].as_str(), Some(""), "no parent window");
        assert_eq!(m.body[1].get("interactive").and_then(Value::as_bool), Some(false));
        let handle = Value::path(&request_path(":1.42", &opts_token(m)));
        Ok((vec![handle], Some((0, results(vec![("uri", Value::str("file:///tmp/out%20one.png"))])))))
    });
    let uri = screenshot(&mut bus).unwrap();
    assert_eq!(file_uri_path(&uri).as_deref(), Some("/tmp/out one.png"));
    assert!(bus.rules.is_empty(), "the Response match is removed");

    let mut refused = Fake::new(|m| {
        let handle = Value::path(&request_path(":1.42", &opts_token(m)));
        Ok((vec![handle], Some((2, results(vec![])))))
    });
    assert!(screenshot(&mut refused).is_err());
}

#[test]
fn file_uris_become_paths() {
    assert_eq!(file_uri_path("file:///home/u/Pictures/Screenshot.png").as_deref(), Some("/home/u/Pictures/Screenshot.png"));
    assert_eq!(file_uri_path("file://localhost/tmp/a%2Bb.png").as_deref(), Some("/tmp/a+b.png"));
    assert_eq!(file_uri_path("file://host/tmp/a.png"), None, "a remote host");
    assert_eq!(file_uri_path("https://x/a.png"), None);
    assert_eq!(file_uri_path("file:///bad%zz"), None);
    assert_eq!(file_uri_path("file:///cut%2"), None);
}

#[test]
fn the_screenshot_is_cropped_to_the_rect() {
    use crate::pixbuf::PixBuf;
    // 4x2: pixel (x, y) has red x and green y.
    let mut d = Vec::new();
    for y in 0..2u8 {
        for x in 0..4u8 {
            d.extend([x, y, 0, 255]);
        }
    }
    let img = PixBuf::from_raw(4, 2, d).unwrap();
    let c = crop_screen(&img, (4, 2), (1, 1, 2, 1)).unwrap();
    assert_eq!(c.as_raw(), &vec![1, 1, 0, 255, 2, 1, 0, 255]);
    // A 2x HiDPI picture of a 2x1 screen.
    let c = crop_screen(&img, (2, 1), (1, 0, 1, 1)).unwrap();
    assert_eq!(c.dimensions(), (1, 1));
    assert_eq!(c.get_pixel(0, 0), [3, 1, 0, 255]);
    // Off the picture: black, still the asked size.
    let c = crop_screen(&img, (4, 2), (3, 1, 2, 2)).unwrap();
    assert_eq!(c.dimensions(), (2, 2));
    assert_eq!(c.get_pixel(0, 0), [3, 1, 0, 255]);
    assert_eq!(c.get_pixel(1, 1), [0, 0, 0, 255]);
    assert!(crop_screen(&img, (4, 2), (0, 0, 0, 1)).is_none());
}
