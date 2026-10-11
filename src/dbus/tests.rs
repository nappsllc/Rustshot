use super::*;

fn round(v: Value) {
    let mut w = Writer::new();
    w.put(&v).unwrap();
    let ty = parse_sig(&v.signature()).unwrap();
    let mut r = Reader::new(&w.buf, 0, true);
    assert_eq!(r.get(&ty[0]).unwrap(), v, "round trip of {}", v.signature());
}

#[test]
fn basic_values_round_trip() {
    for v in [
        Value::Byte(7),
        Value::Bool(true),
        Value::Bool(false),
        Value::I16(-2),
        Value::U16(65000),
        Value::I32(-123456),
        Value::U32(0xDEADBEEF),
        Value::I64(-1 << 40),
        Value::U64(1 << 60),
        Value::F64(1.5),
        Value::str("héllo"),
        Value::path("/org/freedesktop/portal/desktop"),
        Value::Sig("a{sv}".into()),
        Value::Fd(0),
    ] {
        round(v);
    }
}

#[test]
fn containers_round_trip() {
    round(Value::Array("s".into(), vec![Value::str("a"), Value::str("bc")]));
    round(Value::Array("t".into(), vec![]));
    round(Value::dict_sv(vec![("types", Value::U32(1)), ("multiple", Value::Bool(false)), ("token", Value::str("x"))]));
    // a(ua{sv}): the portal's streams.
    let stream = Value::Struct(vec![Value::U32(42), Value::dict_sv(vec![("size", Value::Struct(vec![Value::I32(1920), Value::I32(1080)]))])]);
    round(Value::Array("(ua{sv})".into(), vec![stream.clone(), stream]));
    round(Value::variant(Value::variant(Value::Array("y".into(), vec![Value::Byte(1), Value::Byte(2)]))));
    round(Value::Struct(vec![Value::Byte(1), Value::U64(2), Value::str("z"), Value::I16(3)]));
}

#[test]
fn alignment_follows_the_spec() {
    // A struct starts on 8; a u64 after a byte is padded to 8.
    let mut w = Writer::new();
    w.put(&Value::Struct(vec![Value::Byte(1), Value::U64(2)])).unwrap();
    assert_eq!(w.buf, [1, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0]);
    // An empty array of u64: length 0, then padding to the element's
    // alignment even though there are no elements.
    let mut w = Writer::new();
    w.put(&Value::Byte(9)).unwrap();
    w.put(&Value::Array("t".into(), vec![])).unwrap();
    assert_eq!(w.buf, [9, 0, 0, 0, 0, 0, 0, 0]);
    // Array length excludes the padding before the first element.
    let mut w = Writer::new();
    w.put(&Value::Array("t".into(), vec![Value::U64(5)])).unwrap();
    assert_eq!(&w.buf[..4], &8u32.to_le_bytes());
    assert_eq!(w.buf.len(), 16);
    // Strings: length, bytes, NUL; a signature: one length byte.
    let mut w = Writer::new();
    w.put(&Value::str("ab")).unwrap();
    w.put(&Value::Sig("u".into())).unwrap();
    assert_eq!(w.buf, [2, 0, 0, 0, b'a', b'b', 0, 1, b'u', 0]);
    // A dict entry is 8-aligned.
    let mut w = Writer::new();
    w.put(&Value::dict_sv(vec![("a", Value::Byte(1))])).unwrap();
    assert_eq!(&w.buf[4..8], &[0, 0, 0, 0], "padding to 8 after the length");
    assert_eq!(w.buf[8..14], [1, 0, 0, 0, b'a', 0]);
}

#[test]
fn wrong_element_types_are_refused() {
    let mut w = Writer::new();
    assert!(w.put(&Value::Array("s".into(), vec![Value::U32(1)])).is_err());
}

#[test]
fn messages_round_trip() {
    let mut m = Message::call("org.freedesktop.portal.Desktop", "/org/freedesktop/portal/desktop", "org.freedesktop.portal.ScreenCast", "CreateSession", vec![
        Value::dict_sv(vec![("handle_token", Value::str("t1")), ("session_handle_token", Value::str("s1"))]),
    ]);
    m.flags = NO_REPLY;
    let bytes = m.encode(9).unwrap();
    assert_eq!(&bytes[..4], &[b'l', 1, NO_REPLY, 1]);
    assert_eq!(Message::wire_len(&bytes).unwrap(), Some(bytes.len()));
    let (d, nfd) = Message::decode(&bytes).unwrap();
    assert_eq!(nfd, 0);
    assert_eq!((d.kind, d.serial, d.flags), (Kind::Call, 9, NO_REPLY));
    assert_eq!(d.dest.as_deref(), Some("org.freedesktop.portal.Desktop"));
    assert!(d.is("org.freedesktop.portal.ScreenCast", "CreateSession"));
    assert_eq!(d.body, m.body);
    assert_eq!(d.signature(), "a{sv}");
    // The body starts 8-aligned.
    let fields = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
    let body_at = (16 + fields).next_multiple_of(8);
    assert_eq!(bytes.len() - body_at, u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize);
}

#[test]
fn replies_errors_and_signals_round_trip() {
    let mut call = Message::call("a.b", "/", "a.b", "M", vec![]);
    call.serial = 5;
    call.sender = Some(":1.7".into());
    let r = Message::reply(&call, vec![Value::U32(0)]);
    let (d, _) = Message::decode(&r.encode(6).unwrap()).unwrap();
    assert_eq!((d.kind, d.reply_to, d.dest.as_deref()), (Kind::Return, Some(5), Some(":1.7")));
    let e = Message::error_reply(&call, "org.freedesktop.DBus.Error.Failed", "no");
    let (d, _) = Message::decode(&e.encode(7).unwrap()).unwrap();
    assert_eq!((d.kind, d.error.as_deref(), d.body[0].as_str()), (Kind::Error, Some("org.freedesktop.DBus.Error.Failed"), Some("no")));
    let s = Message::signal("/x", "a.b", "Sig", vec![Value::str("hi")]);
    let (d, _) = Message::decode(&s.encode(8).unwrap()).unwrap();
    assert_eq!((d.kind, d.path.as_deref(), d.member.as_deref()), (Kind::Signal, Some("/x"), Some("Sig")));
}

#[test]
fn big_endian_messages_are_read() {
    // A signal with path "/a", member "M", body u32 0x01020304, built by hand.
    let mut b = vec![b'B', 4, 0, 1, 0, 0, 0, 4, 0, 0, 0, 7, 0, 0, 0, 0];
    b.extend_from_slice(&[1, 1, b'o', 0, 0, 0, 0, 2, b'/', b'a', 0]); // 16..27
    b.extend_from_slice(&[0, 0, 0, 0, 0]); // pad to 32
    b.extend_from_slice(&[3, 1, b's', 0, 0, 0, 0, 1, b'M', 0]); // 32..42
    b.extend_from_slice(&[0; 6]); // pad to 48
    b.extend_from_slice(&[8, 1, b'g', 0, 1, b'u', 0]); // 48..55
    let fields = (b.len() - 16) as u32;
    b[12..16].copy_from_slice(&fields.to_be_bytes());
    b.push(0); // pad to 56
    b.extend_from_slice(&[1, 2, 3, 4]);
    let (m, _) = Message::decode(&b).unwrap();
    assert_eq!((m.kind, m.serial), (Kind::Signal, 7));
    assert_eq!((m.path.as_deref(), m.member.as_deref()), (Some("/a"), Some("M")));
    assert_eq!(m.body, [Value::U32(0x01020304)]);
}

#[test]
fn truncated_and_bad_messages_fail_cleanly() {
    let m = Message::signal("/x", "a.b", "S", vec![Value::str("hello")]);
    let bytes = m.encode(1).unwrap();
    for n in [0, 10, 16, bytes.len() - 1] {
        assert!(Message::decode(&bytes[..n]).is_err(), "{n} bytes");
    }
    let mut bad = bytes.clone();
    bad[0] = b'x';
    assert!(Message::decode(&bad).is_err());
    assert!(parse_sig("a{vs}").is_err(), "a variant key");
    assert!(parse_sig("(").is_err());
    assert!(parse_sig("a").is_err());
    assert!(parse_sig("()").is_err());
    assert_eq!(parse_sig("ua{sv}").unwrap().len(), 2);
}

#[test]
fn file_descriptors_are_taken_by_index() {
    let mut m = Message::signal("/x", "a.b", "S", vec![]);
    m.fds.push(dummy_fd());
    let v = Value::Fd(0);
    assert!(m.take_fd(&Value::Fd(3)).is_err());
    assert!(m.take_fd(&v).is_ok());
    assert!(m.take_fd(&v).is_err(), "taken once");
    // A fd value needs its descriptor; one with it says UNIX_FDS.
    let m = Message::signal("/x", "a.b", "S", vec![Value::Fd(0)]);
    assert!(m.encode(1).is_err());
    let mut m = Message::signal("/x", "a.b", "S", vec![Value::Fd(0)]);
    m.fds.push(dummy_fd());
    let (_, nfd) = Message::decode(&m.encode(1).unwrap()).unwrap();
    assert_eq!(nfd, 1);
}

#[cfg(unix)]
pub(crate) fn dummy_fd() -> Fd {
    std::fs::File::open("/dev/null").unwrap().into()
}

#[cfg(not(unix))]
pub(crate) fn dummy_fd() -> Fd {
    Fd(7)
}

#[test]
fn values_are_looked_up() {
    let d = Value::dict_sv(vec![("restore_token", Value::str("abc")), ("n", Value::U32(3))]);
    assert_eq!(d.get("restore_token").and_then(Value::as_str), Some("abc"));
    assert_eq!(d.get("n").and_then(Value::as_u32), Some(3));
    assert!(d.get("missing").is_none());
    assert_eq!(Value::variant(Value::I32(-1)).as_i32(), Some(-1));
    assert_eq!(Value::variant(Value::Bool(true)).as_bool(), Some(true));
}

#[test]
fn addresses_auth_and_names() {
    assert_eq!(parse_address("unix:path=/run/user/1000/bus"), [Addr::Path("/run/user/1000/bus".into())]);
    assert_eq!(
        parse_address("unix:abstract=/tmp/dbus-Ab%2c,guid=123;tcp:host=x;unix:path=/a%20b"),
        [Addr::Abstract("/tmp/dbus-Ab,".into()), Addr::Path("/a b".into())]
    );
    assert!(parse_address("tcp:host=localhost,port=1").is_empty());
    assert_eq!(auth_line(1000), "AUTH EXTERNAL 31303030\r\n");
    assert_eq!(auth_line(0), "AUTH EXTERNAL 30\r\n");
    assert_eq!(sender_elem(":1.42"), "1_42");
    assert_eq!(
        signal_rule("org.freedesktop.portal.Desktop", "/r", "org.freedesktop.portal.Request", "Response"),
        "type='signal',interface='org.freedesktop.portal.Request',member='Response',path='/r',sender='org.freedesktop.portal.Desktop'"
    );
}

/// The live bus (Linux CI starts `dbus-daemon --session`): list names,
/// a signal to ourselves through a match rule, and a file descriptor
/// passed to ourselves and back.
#[cfg(target_os = "linux")]
#[test]
#[ignore]
fn live_dbus_session_bus() {
    use std::io::{Read, Write};
    if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_none() {
        println!("SKIP: no session bus");
        return;
    }
    let mut c = Conn::session().expect("connect to the session bus");
    let me = c.unique_name().to_string();
    assert!(me.starts_with(':'), "unique name {me}");
    let names = bus_call(&mut c, "ListNames", vec![]).unwrap();
    let list = names.body[0].items();
    assert!(list.iter().any(|v| v.as_str() == Some(me.as_str())), "ListNames has us");
    assert!(list.iter().any(|v| v.as_str() == Some(BUS_NAME)));
    // An unknown method is an error reply, not a hang.
    let err = c.call(Message::call(BUS_NAME, BUS_PATH, BUS_NAME, "NoSuchMethod", vec![]), CALL_TIMEOUT).unwrap_err();
    assert!(err.downcast_ref::<DbusError>().is_some(), "{err:#}");

    // A signal to ourselves.
    add_match(&mut c, &signal_rule("", "/io/github/nappsllc/test", "io.github.nappsllc.Test", "Ping")).unwrap();
    c.send(Message::signal("/io/github/nappsllc/test", "io.github.nappsllc.Test", "Ping", vec![Value::U32(77)])).unwrap();
    let got = c.wait_signal(&|m| m.is("io.github.nappsllc.Test", "Ping"), Duration::from_secs(5)).unwrap().expect("our signal");
    assert_eq!(got.body, [Value::U32(77)]);
    assert_eq!(got.sender.as_deref(), Some(me.as_str()));
    println!("DBUS signal round trip ok as {me}");

    // A descriptor to ourselves: write into the pipe through the received copy.
    let (r, w) = crate::rec::linux::child::pipe().unwrap();
    let mut m = Message::call(&me, "/io/github/nappsllc/test", "io.github.nappsllc.Test", "TakeFd", vec![Value::Fd(0)]);
    m.flags = NO_REPLY;
    m.fds.push(w);
    c.send(m).unwrap();
    let mut call = c.wait_call(Duration::from_secs(5)).unwrap().expect("our call");
    assert!(call.is("io.github.nappsllc.Test", "TakeFd"));
    let v = call.body[0].clone();
    let fd = call.take_fd(&v).unwrap();
    let mut f = std::fs::File::from(fd);
    f.write_all(b"through the bus").unwrap();
    drop(f);
    let mut s = String::new();
    std::fs::File::from(r).read_to_string(&mut s).unwrap();
    assert_eq!(s, "through the bus");
    println!("DBUS fd passing ok");
}
