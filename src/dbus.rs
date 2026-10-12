//! A small hand-written D-Bus client: the session bus over a Unix socket
//! (`DBUS_SESSION_BUS_ADDRESS`: `unix:path=` or `unix:abstract=`), SASL
//! `AUTH EXTERNAL` with the uid, `NEGOTIATE_UNIX_FD`, `Hello`, message
//! marshalling of the basic and container types (arrays, dicts, structs,
//! variants; both byte orders on read, little-endian on write), method
//! calls matched to their replies by serial, signal match rules
//! (`AddMatch`), timeouts on every wait, and file descriptors passed with
//! `SCM_RIGHTS` (type `h`).
//!
//! Used by the xdg-desktop-portal ScreenCast and Camera clients
//! (`rec::linux::portal`) and the StatusNotifierItem tray (`sni`).
//! The marshalling ([`Value`], [`Message`]) is pure and tested everywhere;
//! the transport ([`Conn`]) is Linux-only. Everything that waits for
//! the bus goes through the [`Bus`] trait, so request/response logic can be
//! tested against a scripted fake.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))] // the transport is Linux-only

use anyhow::{anyhow, bail, Result};
use std::time::Duration;

/// A file descriptor carried by a message (an `h` value indexes
/// [`Message::fds`]). Owned: it closes when dropped.
#[cfg(unix)]
pub type Fd = std::os::fd::OwnedFd;
/// Stand-in for a descriptor where there are none (tests on Windows).
#[cfg(not(unix))]
#[derive(Debug)]
pub struct Fd(pub i32);

/// A D-Bus value.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Byte(u8),
    Bool(bool),
    I16(i16),
    U16(u16),
    I32(i32),
    U32(u32),
    I64(i64),
    U64(u64),
    F64(f64),
    Str(String),
    Path(String),
    Sig(String),
    /// `h`: an index into the message's file descriptors.
    Fd(u32),
    /// `a<elem>`: the element signature and the elements.
    Array(String, Vec<Value>),
    /// `a{<key><value>}`: key and value signatures and the entries.
    Dict(String, String, Vec<(Value, Value)>),
    Struct(Vec<Value>),
    Variant(Box<Value>),
}

impl Value {
    pub fn str(s: &str) -> Value {
        Value::Str(s.into())
    }

    pub fn path(s: &str) -> Value {
        Value::Path(s.into())
    }

    pub fn variant(v: Value) -> Value {
        Value::Variant(Box::new(v))
    }

    /// An `a{sv}` from (key, value) pairs (each value wrapped in a variant).
    pub fn dict_sv(items: Vec<(&str, Value)>) -> Value {
        Value::Dict("s".into(), "v".into(), items.into_iter().map(|(k, v)| (Value::str(k), Value::variant(v))).collect())
    }

    /// This value's signature.
    pub fn signature(&self) -> String {
        match self {
            Value::Byte(_) => "y".into(),
            Value::Bool(_) => "b".into(),
            Value::I16(_) => "n".into(),
            Value::U16(_) => "q".into(),
            Value::I32(_) => "i".into(),
            Value::U32(_) => "u".into(),
            Value::I64(_) => "x".into(),
            Value::U64(_) => "t".into(),
            Value::F64(_) => "d".into(),
            Value::Str(_) => "s".into(),
            Value::Path(_) => "o".into(),
            Value::Sig(_) => "g".into(),
            Value::Fd(_) => "h".into(),
            Value::Array(e, _) => format!("a{e}"),
            Value::Dict(k, v, _) => format!("a{{{k}{v}}}"),
            Value::Struct(f) => format!("({})", f.iter().map(Value::signature).collect::<String>()),
            Value::Variant(_) => "v".into(),
        }
    }

    /// The value inside any variants.
    pub fn inner(&self) -> &Value {
        match self {
            Value::Variant(v) => v.inner(),
            v => v,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self.inner() {
            Value::Str(s) | Value::Path(s) | Value::Sig(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_u32(&self) -> Option<u32> {
        match *self.inner() {
            Value::U32(v) => Some(v),
            Value::Byte(v) => Some(u32::from(v)),
            Value::U16(v) => Some(u32::from(v)),
            Value::I32(v) => u32::try_from(v).ok(),
            _ => None,
        }
    }

    pub fn as_i32(&self) -> Option<i32> {
        match *self.inner() {
            Value::I32(v) => Some(v),
            Value::U32(v) => i32::try_from(v).ok(),
            Value::I16(v) => Some(i32::from(v)),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match *self.inner() {
            Value::Bool(b) => Some(b),
            _ => None,
        }
    }

    /// The elements of an array or the fields of a struct.
    pub fn items(&self) -> &[Value] {
        match self.inner() {
            Value::Array(_, v) | Value::Struct(v) => v,
            _ => &[],
        }
    }

    /// The value under string key `key` of a dict (`a{sv}` and the like).
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self.inner() {
            Value::Dict(_, _, e) => e.iter().find(|(k, _)| k.as_str() == Some(key)).map(|(_, v)| v.inner()),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Signatures
// ---------------------------------------------------------------------------

/// A parsed single complete type.
#[derive(Clone, Debug, PartialEq)]
pub enum Ty {
    Byte,
    Bool,
    I16,
    U16,
    I32,
    U32,
    I64,
    U64,
    F64,
    Str,
    Path,
    Sig,
    Fd,
    Variant,
    Array(Box<Ty>),
    Dict(Box<Ty>, Box<Ty>),
    Struct(Vec<Ty>),
}

impl Ty {
    fn align(&self) -> usize {
        match self {
            Ty::Byte | Ty::Sig | Ty::Variant => 1,
            Ty::I16 | Ty::U16 => 2,
            Ty::Bool | Ty::I32 | Ty::U32 | Ty::Str | Ty::Path | Ty::Fd | Ty::Array(_) | Ty::Dict(..) => 4,
            Ty::I64 | Ty::U64 | Ty::F64 | Ty::Struct(_) => 8,
        }
    }

    fn sig(&self) -> String {
        match self {
            Ty::Byte => "y".into(),
            Ty::Bool => "b".into(),
            Ty::I16 => "n".into(),
            Ty::U16 => "q".into(),
            Ty::I32 => "i".into(),
            Ty::U32 => "u".into(),
            Ty::I64 => "x".into(),
            Ty::U64 => "t".into(),
            Ty::F64 => "d".into(),
            Ty::Str => "s".into(),
            Ty::Path => "o".into(),
            Ty::Sig => "g".into(),
            Ty::Fd => "h".into(),
            Ty::Variant => "v".into(),
            Ty::Array(e) => format!("a{}", e.sig()),
            Ty::Dict(k, v) => format!("a{{{}{}}}", k.sig(), v.sig()),
            Ty::Struct(f) => format!("({})", f.iter().map(Ty::sig).collect::<String>()),
        }
    }

    fn is_basic(&self) -> bool {
        !matches!(self, Ty::Variant | Ty::Array(_) | Ty::Dict(..) | Ty::Struct(_))
    }
}

const MAX_DEPTH: usize = 32;

/// Parse a signature into its complete types.
pub fn parse_sig(s: &str) -> Result<Vec<Ty>> {
    if s.len() > 255 {
        bail!("signature too long");
    }
    let b = s.as_bytes();
    let mut i = 0;
    let mut out = Vec::new();
    while i < b.len() {
        out.push(parse_one(b, &mut i, 0)?);
    }
    Ok(out)
}

fn parse_one(b: &[u8], i: &mut usize, depth: usize) -> Result<Ty> {
    if depth > MAX_DEPTH {
        bail!("signature nested too deeply");
    }
    let Some(&c) = b.get(*i) else { bail!("signature ends early") };
    *i += 1;
    Ok(match c {
        b'y' => Ty::Byte,
        b'b' => Ty::Bool,
        b'n' => Ty::I16,
        b'q' => Ty::U16,
        b'i' => Ty::I32,
        b'u' => Ty::U32,
        b'x' => Ty::I64,
        b't' => Ty::U64,
        b'd' => Ty::F64,
        b's' => Ty::Str,
        b'o' => Ty::Path,
        b'g' => Ty::Sig,
        b'h' => Ty::Fd,
        b'v' => Ty::Variant,
        b'a' => {
            if b.get(*i) == Some(&b'{') {
                *i += 1;
                let k = parse_one(b, i, depth + 1)?;
                if !k.is_basic() {
                    bail!("dict key must be a basic type");
                }
                let v = parse_one(b, i, depth + 1)?;
                if b.get(*i) != Some(&b'}') {
                    bail!("dict entry not closed");
                }
                *i += 1;
                Ty::Dict(Box::new(k), Box::new(v))
            } else {
                Ty::Array(Box::new(parse_one(b, i, depth + 1)?))
            }
        }
        b'(' => {
            let mut f = Vec::new();
            while b.get(*i) != Some(&b')') {
                if *i >= b.len() {
                    bail!("struct not closed");
                }
                f.push(parse_one(b, i, depth + 1)?);
            }
            *i += 1;
            if f.is_empty() {
                bail!("empty struct");
            }
            Ty::Struct(f)
        }
        c => bail!("bad signature character {:?}", c as char),
    })
}

// ---------------------------------------------------------------------------
// Marshalling
// ---------------------------------------------------------------------------

/// Writes values little-endian; offsets (for alignment) count from the
/// start of `buf`, which must start at an 8-aligned message offset.
pub struct Writer {
    pub buf: Vec<u8>,
}

impl Writer {
    pub fn new() -> Writer {
        Writer { buf: Vec::new() }
    }

    fn pad(&mut self, a: usize) {
        while !self.buf.len().is_multiple_of(a) {
            self.buf.push(0);
        }
    }

    fn u32(&mut self, v: u32) {
        self.pad(4);
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    fn string(&mut self, s: &str) {
        self.u32(s.len() as u32);
        self.buf.extend_from_slice(s.as_bytes());
        self.buf.push(0);
    }

    fn sig(&mut self, s: &str) {
        self.buf.push(s.len() as u8);
        self.buf.extend_from_slice(s.as_bytes());
        self.buf.push(0);
    }

    /// Append `v`.
    pub fn put(&mut self, v: &Value) -> Result<()> {
        match v {
            Value::Byte(b) => self.buf.push(*b),
            Value::Bool(b) => self.u32(u32::from(*b)),
            Value::I16(x) => {
                self.pad(2);
                self.buf.extend_from_slice(&x.to_le_bytes());
            }
            Value::U16(x) => {
                self.pad(2);
                self.buf.extend_from_slice(&x.to_le_bytes());
            }
            Value::I32(x) => {
                self.pad(4);
                self.buf.extend_from_slice(&x.to_le_bytes());
            }
            Value::U32(x) | Value::Fd(x) => self.u32(*x),
            Value::I64(x) => {
                self.pad(8);
                self.buf.extend_from_slice(&x.to_le_bytes());
            }
            Value::U64(x) => {
                self.pad(8);
                self.buf.extend_from_slice(&x.to_le_bytes());
            }
            Value::F64(x) => {
                self.pad(8);
                self.buf.extend_from_slice(&x.to_le_bytes());
            }
            Value::Str(s) | Value::Path(s) => self.string(s),
            Value::Sig(s) => self.sig(s),
            Value::Array(e, items) => {
                let ty = parse_sig(e)?;
                let [ty] = ty.as_slice() else { bail!("array element signature {e:?}") };
                self.u32(0);
                let len_at = self.buf.len() - 4;
                self.pad(ty.align());
                let start = self.buf.len();
                for it in items {
                    if it.signature() != *e {
                        bail!("array of {e} holds a {}", it.signature());
                    }
                    self.put(it)?;
                }
                let n = (self.buf.len() - start) as u32;
                self.buf[len_at..len_at + 4].copy_from_slice(&n.to_le_bytes());
            }
            Value::Dict(k, val, items) => {
                self.u32(0);
                let len_at = self.buf.len() - 4;
                self.pad(8);
                let start = self.buf.len();
                for (ik, iv) in items {
                    if ik.signature() != *k || iv.signature() != *val {
                        bail!("dict a{{{k}{val}}} holds {}/{}", ik.signature(), iv.signature());
                    }
                    self.pad(8);
                    self.put(ik)?;
                    self.put(iv)?;
                }
                let n = (self.buf.len() - start) as u32;
                self.buf[len_at..len_at + 4].copy_from_slice(&n.to_le_bytes());
            }
            Value::Struct(f) => {
                self.pad(8);
                for x in f {
                    self.put(x)?;
                }
            }
            Value::Variant(inner) => {
                self.sig(&inner.signature());
                self.put(inner)?;
            }
        }
        Ok(())
    }
}

impl Default for Writer {
    fn default() -> Self {
        Writer::new()
    }
}

/// Reads values of either byte order; `base` is the message offset of
/// `buf[0]` (for alignment).
pub struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
    base: usize,
    le: bool,
}

const MAX_ARRAY: usize = 64 << 20;

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8], base: usize, le: bool) -> Reader<'a> {
        Reader { buf, pos: 0, base, le }
    }

    fn align(&mut self, a: usize) -> Result<()> {
        let abs = self.base + self.pos;
        let p = abs.next_multiple_of(a) - abs;
        if self.pos + p > self.buf.len() {
            bail!("message truncated");
        }
        if self.buf[self.pos..self.pos + p].iter().any(|&b| b != 0) {
            bail!("non-zero padding");
        }
        self.pos += p;
        Ok(())
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        if self.pos + n > self.buf.len() {
            bail!("message truncated");
        }
        let s = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    fn fixed<const N: usize>(&mut self) -> Result<[u8; N]> {
        self.align(N)?;
        let mut a: [u8; N] = self.take(N)?.try_into().expect("N bytes");
        if !self.le {
            a.reverse();
        }
        Ok(a)
    }

    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.fixed::<4>()?))
    }

    fn string(&mut self) -> Result<String> {
        let n = self.u32()? as usize;
        let s = self.take(n + 1)?;
        if s[n] != 0 {
            bail!("string not terminated");
        }
        String::from_utf8(s[..n].to_vec()).map_err(|_| anyhow!("string is not UTF-8"))
    }

    fn sig(&mut self) -> Result<String> {
        let n = self.take(1)?[0] as usize;
        let s = self.take(n + 1)?;
        if s[n] != 0 {
            bail!("signature not terminated");
        }
        String::from_utf8(s[..n].to_vec()).map_err(|_| anyhow!("bad signature"))
    }

    /// Read one value of type `ty`.
    pub fn get(&mut self, ty: &Ty) -> Result<Value> {
        self.get_at(ty, 0)
    }

    fn get_at(&mut self, ty: &Ty, depth: usize) -> Result<Value> {
        if depth > MAX_DEPTH {
            bail!("value nested too deeply");
        }
        Ok(match ty {
            Ty::Byte => Value::Byte(self.take(1)?[0]),
            Ty::Bool => match self.u32()? {
                0 => Value::Bool(false),
                1 => Value::Bool(true),
                v => bail!("bad boolean {v}"),
            },
            Ty::I16 => Value::I16(i16::from_le_bytes(self.fixed::<2>()?)),
            Ty::U16 => Value::U16(u16::from_le_bytes(self.fixed::<2>()?)),
            Ty::I32 => Value::I32(i32::from_le_bytes(self.fixed::<4>()?)),
            Ty::U32 => Value::U32(self.u32()?),
            Ty::Fd => Value::Fd(self.u32()?),
            Ty::I64 => Value::I64(i64::from_le_bytes(self.fixed::<8>()?)),
            Ty::U64 => Value::U64(u64::from_le_bytes(self.fixed::<8>()?)),
            Ty::F64 => Value::F64(f64::from_le_bytes(self.fixed::<8>()?)),
            Ty::Str => Value::Str(self.string()?),
            Ty::Path => Value::Path(self.string()?),
            Ty::Sig => Value::Sig(self.sig()?),
            Ty::Variant => {
                let s = self.sig()?;
                let t = parse_sig(&s)?;
                let [t] = t.as_slice() else { bail!("variant signature {s:?}") };
                Value::Variant(Box::new(self.get_at(t, depth + 1)?))
            }
            Ty::Array(e) => {
                let n = self.u32()? as usize;
                if n > MAX_ARRAY {
                    bail!("array too long");
                }
                self.align(e.align())?;
                let end = self.pos + n;
                if end > self.buf.len() {
                    bail!("message truncated");
                }
                let mut items = Vec::new();
                while self.pos < end {
                    items.push(self.get_at(e, depth + 1)?);
                }
                if self.pos != end {
                    bail!("array length mismatch");
                }
                Value::Array(e.sig(), items)
            }
            Ty::Dict(k, v) => {
                let n = self.u32()? as usize;
                if n > MAX_ARRAY {
                    bail!("array too long");
                }
                self.align(8)?;
                let end = self.pos + n;
                if end > self.buf.len() {
                    bail!("message truncated");
                }
                let mut items = Vec::new();
                while self.pos < end {
                    self.align(8)?;
                    let key = self.get_at(k, depth + 1)?;
                    let val = self.get_at(v, depth + 1)?;
                    items.push((key, val));
                }
                if self.pos != end {
                    bail!("dict length mismatch");
                }
                Value::Dict(k.sig(), v.sig(), items)
            }
            Ty::Struct(f) => {
                self.align(8)?;
                let mut out = Vec::with_capacity(f.len());
                for t in f {
                    out.push(self.get_at(t, depth + 1)?);
                }
                Value::Struct(out)
            }
        })
    }
}

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Call = 1,
    Return = 2,
    Error = 3,
    Signal = 4,
}

/// Message flag: the caller wants no reply.
pub const NO_REPLY: u8 = 1;

/// A D-Bus message.
#[derive(Debug)]
pub struct Message {
    pub kind: Kind,
    pub flags: u8,
    pub serial: u32,
    pub path: Option<String>,
    pub iface: Option<String>,
    pub member: Option<String>,
    pub error: Option<String>,
    pub reply_to: Option<u32>,
    pub dest: Option<String>,
    pub sender: Option<String>,
    pub body: Vec<Value>,
    /// Descriptors sent with it (`h` values index them); a taken one
    /// leaves `None`, so the other indices stay valid.
    pub fds: Vec<Option<Fd>>,
}

const MAX_MESSAGE: usize = 128 << 20;

impl Message {
    fn new(kind: Kind) -> Message {
        Message {
            kind,
            flags: 0,
            serial: 0,
            path: None,
            iface: None,
            member: None,
            error: None,
            reply_to: None,
            dest: None,
            sender: None,
            body: Vec::new(),
            fds: Vec::new(),
        }
    }

    /// A method call.
    pub fn call(dest: &str, path: &str, iface: &str, member: &str, body: Vec<Value>) -> Message {
        let mut m = Message::new(Kind::Call);
        m.dest = Some(dest.into());
        m.path = Some(path.into());
        m.iface = Some(iface.into());
        m.member = Some(member.into());
        m.body = body;
        m
    }

    /// A broadcast signal.
    #[allow(dead_code)] // tests; the StatusNotifierItem tray
    pub fn signal(path: &str, iface: &str, member: &str, body: Vec<Value>) -> Message {
        let mut m = Message::new(Kind::Signal);
        m.path = Some(path.into());
        m.iface = Some(iface.into());
        m.member = Some(member.into());
        m.body = body;
        m
    }

    /// A reply to `to` (a method return).
    #[allow(dead_code)] // tests; the StatusNotifierItem tray
    pub fn reply(to: &Message, body: Vec<Value>) -> Message {
        let mut m = Message::new(Kind::Return);
        m.reply_to = Some(to.serial);
        m.dest = to.sender.clone();
        m.body = body;
        m
    }

    /// An error reply to `to`.
    #[allow(dead_code)] // tests; the StatusNotifierItem tray
    pub fn error_reply(to: &Message, name: &str, text: &str) -> Message {
        let mut m = Message::new(Kind::Error);
        m.reply_to = Some(to.serial);
        m.dest = to.sender.clone();
        m.error = Some(name.into());
        m.body = vec![Value::str(text)];
        m
    }

    /// The body's signature.
    pub fn signature(&self) -> String {
        self.body.iter().map(Value::signature).collect()
    }

    pub fn is(&self, iface: &str, member: &str) -> bool {
        self.iface.as_deref() == Some(iface) && self.member.as_deref() == Some(member)
    }

    /// Attach `f`; its index (the `h` value that refers to it).
    #[allow(dead_code)] // tests; the StatusNotifierItem tray
    pub fn push_fd(&mut self, f: Fd) -> u32 {
        self.fds.push(Some(f));
        (self.fds.len() - 1) as u32
    }

    /// Take descriptor `v` (an `h` value) out of the message; the others
    /// keep their indices.
    pub fn take_fd(&mut self, v: &Value) -> Result<Fd> {
        let Value::Fd(i) = v.inner() else { bail!("not a file descriptor") };
        let i = *i as usize;
        match self.fds.get_mut(i) {
            None => bail!("the message carries no file descriptor {i}"),
            Some(slot) => slot.take().ok_or_else(|| anyhow!("file descriptor {i} was already taken")),
        }
    }

    /// The wire form (little-endian), with `serial`.
    pub fn encode(&self, serial: u32) -> Result<Vec<u8>> {
        let mut body = Writer::new();
        for v in &self.body {
            body.put(v)?;
        }
        let mut fields: Vec<Value> = Vec::new();
        let mut field = |code: u8, v: Value| fields.push(Value::Struct(vec![Value::Byte(code), Value::variant(v)]));
        if let Some(p) = &self.path {
            field(1, Value::Path(p.clone()));
        }
        if let Some(s) = &self.iface {
            field(2, Value::Str(s.clone()));
        }
        if let Some(s) = &self.member {
            field(3, Value::Str(s.clone()));
        }
        if let Some(s) = &self.error {
            field(4, Value::Str(s.clone()));
        }
        if let Some(r) = self.reply_to {
            field(5, Value::U32(r));
        }
        if let Some(s) = &self.dest {
            field(6, Value::Str(s.clone()));
        }
        if let Some(s) = &self.sender {
            field(7, Value::Str(s.clone()));
        }
        let sig = self.signature();
        if !sig.is_empty() {
            field(8, Value::Sig(sig));
        }
        if self.body.iter().any(has_fd) && self.fds.is_empty() {
            bail!("a file descriptor value without a descriptor");
        }
        if !self.fds.is_empty() {
            field(9, Value::U32(self.fds.len() as u32));
        }
        let mut w = Writer::new();
        w.buf.extend_from_slice(&[b'l', self.kind as u8, self.flags, 1]);
        w.buf.extend_from_slice(&(body.buf.len() as u32).to_le_bytes());
        w.buf.extend_from_slice(&serial.to_le_bytes());
        w.put(&Value::Array("(yv)".into(), fields))?;
        w.pad(8);
        w.buf.extend_from_slice(&body.buf);
        if w.buf.len() > MAX_MESSAGE {
            bail!("message too large");
        }
        Ok(w.buf)
    }

    /// The length of the message at the start of `buf`, once its first 16
    /// bytes are there.
    pub fn wire_len(buf: &[u8]) -> Result<Option<usize>> {
        if buf.len() < 16 {
            return Ok(None);
        }
        let le = match buf[0] {
            b'l' => true,
            b'B' => false,
            c => bail!("bad byte order mark {c:#x}"),
        };
        let rd = |o: usize| {
            let a: [u8; 4] = buf[o..o + 4].try_into().expect("4 bytes");
            if le { u32::from_le_bytes(a) } else { u32::from_be_bytes(a) }
        };
        let (body, fields) = (rd(4) as usize, rd(12) as usize);
        let n = (16 + fields).next_multiple_of(8) + body;
        if n > MAX_MESSAGE {
            bail!("message too large");
        }
        Ok(Some(n))
    }

    /// Parse one whole message (no descriptors attached yet); also the
    /// number of descriptors it says it carries.
    pub fn decode(buf: &[u8]) -> Result<(Message, u32)> {
        let (mut m, h) = Message::header(buf)?;
        let le = buf[0] == b'l';
        let mut br = Reader::new(&buf[h.body_at..h.end], h.body_at, le);
        for t in parse_sig(&h.sig)? {
            m.body.push(br.get(&t)?);
        }
        if br.pos != h.end - h.body_at {
            bail!("body longer than its signature");
        }
        Ok((m, h.nfd))
    }

    /// The number of descriptors the (whole) message in `buf` says it
    /// carries, when at least its header can be read.
    pub fn declared_fds(buf: &[u8]) -> Option<u32> {
        Message::header(buf).ok().map(|(_, h)| h.nfd)
    }

    /// The header of the whole message at the start of `buf` (the message
    /// without its body) and where its body is.
    fn header(buf: &[u8]) -> Result<(Message, Header)> {
        let Some(n) = Message::wire_len(buf)? else { bail!("message truncated") };
        if buf.len() < n {
            bail!("message truncated");
        }
        let le = buf[0] == b'l';
        let kind = match buf[1] {
            1 => Kind::Call,
            2 => Kind::Return,
            3 => Kind::Error,
            4 => Kind::Signal,
            k => bail!("unknown message type {k}"),
        };
        if buf[3] != 1 {
            bail!("unknown protocol version {}", buf[3]);
        }
        let mut r = Reader::new(&buf[..n], 0, le);
        r.pos = 4;
        let body_len = r.u32()? as usize;
        let serial = r.u32()?;
        let fields = r.get(&Ty::Array(Box::new(Ty::Struct(vec![Ty::Byte, Ty::Variant]))))?;
        let mut m = Message::new(kind);
        m.flags = buf[2];
        m.serial = serial;
        let mut sig = String::new();
        let mut nfd = 0;
        for f in fields.items() {
            let [Value::Byte(code), v] = f.items() else { continue };
            let v = v.inner();
            match code {
                1 => m.path = v.as_str().map(Into::into),
                2 => m.iface = v.as_str().map(Into::into),
                3 => m.member = v.as_str().map(Into::into),
                4 => m.error = v.as_str().map(Into::into),
                5 => m.reply_to = v.as_u32(),
                6 => m.dest = v.as_str().map(Into::into),
                7 => m.sender = v.as_str().map(Into::into),
                8 => sig = v.as_str().unwrap_or_default().into(),
                9 => nfd = v.as_u32().unwrap_or(0),
                _ => {}
            }
        }
        r.align(8)?;
        if r.pos + body_len != n {
            bail!("body length mismatch");
        }
        Ok((m, Header { sig, nfd, body_at: r.pos, end: n }))
    }
}

/// Where a message's body is and what it holds.
struct Header {
    sig: String,
    nfd: u32,
    body_at: usize,
    end: usize,
}

fn has_fd(v: &Value) -> bool {
    match v {
        Value::Fd(_) => true,
        Value::Array(_, i) | Value::Struct(i) => i.iter().any(has_fd),
        Value::Dict(_, _, e) => e.iter().any(|(k, v)| has_fd(k) || has_fd(v)),
        Value::Variant(v) => has_fd(v),
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Bus
// ---------------------------------------------------------------------------

/// An error reply (`org.freedesktop.DBus.Error.*` and the like).
#[derive(Debug)]
pub struct DbusError {
    pub name: String,
    pub text: String,
}

impl std::fmt::Display for DbusError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.text.is_empty() { f.write_str(&self.name) } else { write!(f, "{}: {}", self.name, self.text) }
    }
}

impl std::error::Error for DbusError {}

/// What the portal clients need from a bus (the real [`Conn`], or a
/// scripted fake in tests).
pub trait Bus {
    /// Our unique name (`:1.42`).
    fn unique_name(&self) -> &str;
    /// Send `m` (a method call) and wait up to `timeout` for its reply; an
    /// error reply is a [`DbusError`]. Signals that arrive meanwhile are
    /// kept for [`Bus::wait_signal`].
    fn call(&mut self, m: Message, timeout: Duration) -> Result<Message>;
    /// Send `m` without waiting for anything; its serial.
    fn send(&mut self, m: Message) -> Result<u32>;
    /// The first signal (kept or arriving) that `want` accepts, waiting up
    /// to `timeout`; `None` when none came.
    fn wait_signal(&mut self, want: &dyn Fn(&Message) -> bool, timeout: Duration) -> Result<Option<Message>>;
}

pub const BUS_NAME: &str = "org.freedesktop.DBus";
pub const BUS_PATH: &str = "/org/freedesktop/DBus";
/// How long an ordinary call may take.
pub const CALL_TIMEOUT: Duration = Duration::from_secs(5);

/// Call a method of the bus itself.
pub fn bus_call(b: &mut dyn Bus, member: &str, body: Vec<Value>) -> Result<Message> {
    b.call(Message::call(BUS_NAME, BUS_PATH, BUS_NAME, member, body), CALL_TIMEOUT)
}

/// Ask the bus for the signals `rule` matches.
pub fn add_match(b: &mut dyn Bus, rule: &str) -> Result<()> {
    bus_call(b, "AddMatch", vec![Value::str(rule)]).map(|_| ())
}

pub fn remove_match(b: &mut dyn Bus, rule: &str) -> Result<()> {
    bus_call(b, "RemoveMatch", vec![Value::str(rule)]).map(|_| ())
}

/// `org.freedesktop.DBus.Properties.Get`.
pub fn get_property(b: &mut dyn Bus, dest: &str, path: &str, iface: &str, prop: &str) -> Result<Value> {
    let m = Message::call(dest, path, "org.freedesktop.DBus.Properties", "Get", vec![Value::str(iface), Value::str(prop)]);
    let r = b.call(m, CALL_TIMEOUT)?;
    r.body.into_iter().next().map(|v| v.inner().clone()).ok_or_else(|| anyhow!("{iface}.{prop}: empty reply"))
}

/// A match rule for one signal (any sender when `sender` is empty).
pub fn signal_rule(sender: &str, path: &str, iface: &str, member: &str) -> String {
    let mut r = format!("type='signal',interface='{iface}',member='{member}'");
    if !path.is_empty() {
        r.push_str(&format!(",path='{path}'"));
    }
    if !sender.is_empty() {
        r.push_str(&format!(",sender='{sender}'"));
    }
    r
}

// ---------------------------------------------------------------------------
// Addresses and authentication (pure)
// ---------------------------------------------------------------------------

/// A Unix socket address from a D-Bus address string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Addr {
    Path(String),
    Abstract(String),
}

/// The Unix socket addresses in `s` (`;`-separated, `unix:path=…` or
/// `unix:abstract=…`, values %-escaped); others are skipped.
pub fn parse_address(s: &str) -> Vec<Addr> {
    let mut out = Vec::new();
    for a in s.split(';') {
        let Some(rest) = a.strip_prefix("unix:") else { continue };
        for kv in rest.split(',') {
            let Some((k, v)) = kv.split_once('=') else { continue };
            let v = unescape(v);
            match k {
                "path" => out.push(Addr::Path(v)),
                "abstract" => out.push(Addr::Abstract(v)),
                _ => {}
            }
        }
    }
    out
}

fn unescape(v: &str) -> String {
    let b = v.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = |c: u8| (c as char).to_digit(16);
        if b[i] == b'%'
            && i + 2 < b.len()
            && let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2]))
        {
            out.push((h * 16 + l) as u8);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The `AUTH EXTERNAL` line for `uid` (its decimal digits, hex-encoded).
pub fn auth_line(uid: u32) -> String {
    let hex: String = uid.to_string().bytes().map(|c| format!("{c:02x}")).collect();
    format!("AUTH EXTERNAL {hex}\r\n")
}

/// The object-path element for a unique name: `:1.42` → `1_42`.
pub fn sender_elem(unique: &str) -> String {
    unique.trim_start_matches(':').replace('.', "_")
}

// ---------------------------------------------------------------------------
// Transport (Linux)
// ---------------------------------------------------------------------------

#[cfg(target_os = "linux")]
pub use conn::Conn;

#[cfg(target_os = "linux")]
mod conn {
    use super::*;
    use core::ffi::{c_int, c_short, c_void};
    use std::collections::VecDeque;
    use std::io::{Read, Write};
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
    use std::os::unix::net::UnixStream;
    use std::time::Instant;

    #[repr(C)]
    struct IoVec {
        base: *mut c_void,
        len: usize,
    }

    #[repr(C)]
    struct MsgHdr {
        name: *mut c_void,
        namelen: u32,
        iov: *mut IoVec,
        iovlen: usize,
        control: *mut c_void,
        controllen: usize,
        flags: c_int,
    }

    #[repr(C)]
    struct PollFd {
        fd: c_int,
        events: c_short,
        revents: c_short,
    }

    unsafe extern "C" {
        fn sendmsg(fd: c_int, msg: *const MsgHdr, flags: c_int) -> isize;
        fn recvmsg(fd: c_int, msg: *mut MsgHdr, flags: c_int) -> isize;
        fn poll(fds: *mut PollFd, n: u64, timeout: c_int) -> c_int;
        fn getuid() -> u32;
    }

    const SOL_SOCKET: c_int = 1;
    const SCM_RIGHTS: c_int = 1;
    const MSG_NOSIGNAL: c_int = 0x4000;
    const MSG_CMSG_CLOEXEC: c_int = 0x4000_0000;
    const MSG_CTRUNC: c_int = 0x8;
    const POLLIN: c_short = 1;
    const CMSG_HDR: usize = 16; // cmsg_len (size_t), level, type
    const MAX_FDS: usize = 16;
    /// Signals kept for `wait_signal` at most (oldest dropped).
    const KEEP: usize = 256;

    /// A connection to the session bus.
    pub struct Conn {
        sock: UnixStream,
        serial: u32,
        name: String,
        /// Bytes read but not yet a whole message.
        inbuf: Vec<u8>,
        /// Descriptors received, not yet given to a message.
        infds: VecDeque<OwnedFd>,
        /// Signals (and calls to us) not yet asked for.
        kept: VecDeque<Message>,
        /// Why the connection can no longer be used (the stream lost its
        /// framing, or descriptors were cut off): every later use fails.
        broken: Option<String>,
    }

    impl Conn {
        /// Connect to the session bus (`DBUS_SESSION_BUS_ADDRESS`, else
        /// `$XDG_RUNTIME_DIR/bus`), authenticate and say `Hello`.
        pub fn session() -> Result<Conn> {
            Conn::session_within(CALL_TIMEOUT)
        }

        /// [`Conn::session`], each step (connecting, authenticating,
        /// `Hello`) given `timeout` at most.
        pub fn session_within(timeout: Duration) -> Result<Conn> {
            let addr = std::env::var("DBUS_SESSION_BUS_ADDRESS").ok().filter(|s| !s.is_empty());
            let addrs = match addr {
                Some(a) => parse_address(&a),
                None => std::env::var("XDG_RUNTIME_DIR").map(|d| vec![Addr::Path(format!("{d}/bus"))]).unwrap_or_default(),
            };
            if addrs.is_empty() {
                bail!("no D-Bus session bus (DBUS_SESSION_BUS_ADDRESS is not set)");
            }
            let mut last = anyhow!("no usable session bus address");
            for a in &addrs {
                match Conn::connect(a, timeout) {
                    Ok(c) => return Ok(c),
                    Err(e) => last = e,
                }
            }
            Err(last)
        }

        /// Connect to `a`, authenticate and say `Hello`.
        pub fn connect(a: &Addr, timeout: Duration) -> Result<Conn> {
            let sock = match a {
                Addr::Path(p) => UnixStream::connect(p).map_err(|e| anyhow!("connect to the session bus {p}: {e}"))?,
                Addr::Abstract(n) => {
                    use std::os::linux::net::SocketAddrExt;
                    let sa = std::os::unix::net::SocketAddr::from_abstract_name(n.as_bytes())?;
                    UnixStream::connect_addr(&sa).map_err(|e| anyhow!("connect to the session bus @{n}: {e}"))?
                }
            };
            sock.set_read_timeout(Some(timeout))?;
            sock.set_write_timeout(Some(CALL_TIMEOUT))?;
            let mut c = Conn::over(sock);
            c.auth()?;
            c.sock.set_read_timeout(None)?;
            let r = c.call(Message::call(BUS_NAME, BUS_PATH, BUS_NAME, "Hello", Vec::new()), timeout)?;
            c.name = r.body.first().and_then(|v| v.as_str()).ok_or_else(|| anyhow!("Hello: no name"))?.into();
            Ok(c)
        }

        /// A connection over `sock` before authentication (tests: a fake
        /// bus on the other end of a socket pair).
        pub(crate) fn over(sock: UnixStream) -> Conn {
            Conn {
                sock,
                serial: 0,
                name: String::new(),
                inbuf: Vec::new(),
                infds: VecDeque::new(),
                kept: VecDeque::new(),
                broken: None,
            }
        }

        /// Mark the connection unusable for `why`; the error to return.
        fn fail(&mut self, why: String) -> anyhow::Error {
            self.inbuf.clear();
            self.infds.clear();
            self.broken = Some(why.clone());
            anyhow!(why)
        }

        /// Why the connection broke, once it did.
        pub fn broken(&self) -> Option<&str> {
            self.broken.as_deref()
        }

        fn check(&self) -> Result<()> {
            match &self.broken {
                Some(why) => bail!("the session bus connection is broken ({why})"),
                None => Ok(()),
            }
        }

        fn line(&mut self) -> Result<String> {
            let mut out = Vec::new();
            let mut b = [0u8; 1];
            while !out.ends_with(b"\r\n") {
                if self.sock.read(&mut b)? == 0 {
                    bail!("the bus closed the connection during authentication");
                }
                out.push(b[0]);
                if out.len() > 4096 {
                    bail!("authentication line too long");
                }
            }
            out.truncate(out.len() - 2);
            Ok(String::from_utf8_lossy(&out).into_owned())
        }

        fn auth(&mut self) -> Result<()> {
            // SAFETY: getuid cannot fail.
            let uid = unsafe { getuid() };
            self.sock.write_all(b"\0")?;
            self.sock.write_all(auth_line(uid).as_bytes())?;
            let l = self.line()?;
            if !l.starts_with("OK ") {
                bail!("the session bus refused EXTERNAL authentication ({l})");
            }
            self.sock.write_all(b"NEGOTIATE_UNIX_FD\r\n")?;
            let l = self.line()?;
            if !l.starts_with("AGREE_UNIX_FD") {
                eprintln!("rustshot: the session bus does not pass file descriptors ({l})");
            }
            self.sock.write_all(b"BEGIN\r\n")?;
            Ok(())
        }

        fn next_serial(&mut self) -> u32 {
            self.serial = self.serial.wrapping_add(1).max(1);
            self.serial
        }

        fn write(&mut self, mut m: Message) -> Result<u32> {
            self.check()?;
            let serial = self.next_serial();
            if m.fds.len() > MAX_FDS {
                bail!("too many file descriptors in one message");
            }
            let bytes = m.encode(serial)?;
            let Some(fds) = m.fds.iter().map(|f| f.as_ref().map(|f| f.as_raw_fd())).collect::<Option<Vec<RawFd>>>() else {
                bail!("a file descriptor of the message was taken");
            };
            self.write_raw(&bytes, &fds)?;
            m.fds.clear(); // ours were sent as copies; close them here
            Ok(serial)
        }

        /// Send `bytes`, `fds` with the first of them (tests: a fake bus
        /// writing whatever it likes).
        pub(crate) fn write_raw(&mut self, bytes: &[u8], fds: &[RawFd]) -> Result<()> {
            if fds.len() > MAX_FDS {
                bail!("too many file descriptors in one message");
            }
            let mut sent = 0;
            while sent < bytes.len() {
                let mut iov = IoVec { base: bytes[sent..].as_ptr() as *mut c_void, len: bytes.len() - sent };
                let mut ctl = vec![0u64; (CMSG_HDR + MAX_FDS * 4).div_ceil(8)];
                let mut h =
                    MsgHdr { name: core::ptr::null_mut(), namelen: 0, iov: &mut iov, iovlen: 1, control: core::ptr::null_mut(), controllen: 0, flags: 0 };
                if sent == 0 && !fds.is_empty() {
                    let len = CMSG_HDR + fds.len() * 4;
                    let p = ctl.as_mut_ptr() as *mut u8;
                    // SAFETY: `ctl` holds a header and up to MAX_FDS descriptors.
                    unsafe {
                        (p as *mut usize).write(len);
                        (p.add(8) as *mut c_int).write(SOL_SOCKET);
                        (p.add(12) as *mut c_int).write(SCM_RIGHTS);
                        for (i, fd) in fds.iter().enumerate() {
                            (p.add(CMSG_HDR + i * 4) as *mut c_int).write(*fd);
                        }
                    }
                    h.control = p as *mut c_void;
                    h.controllen = len.next_multiple_of(8);
                }
                // SAFETY: the header points at live buffers.
                let n = unsafe { sendmsg(self.sock.as_raw_fd(), &h, MSG_NOSIGNAL) };
                if n < 0 {
                    let e = std::io::Error::last_os_error();
                    if e.kind() == std::io::ErrorKind::Interrupted {
                        continue;
                    }
                    return Err(anyhow!("write to the session bus: {e}"));
                }
                sent += n as usize;
            }
            Ok(())
        }

        /// Wait up to `until` for readable data; false on timeout.
        fn readable(&self, until: Instant) -> Result<bool> {
            loop {
                let left = until.saturating_duration_since(Instant::now());
                let ms = left.as_millis().min(i32::MAX as u128) as c_int;
                let mut p = PollFd { fd: self.sock.as_raw_fd(), events: POLLIN, revents: 0 };
                // SAFETY: one valid pollfd.
                let r = unsafe { poll(&mut p, 1, ms) };
                if r < 0 {
                    let e = std::io::Error::last_os_error();
                    if e.kind() == std::io::ErrorKind::Interrupted {
                        continue;
                    }
                    bail!("poll the session bus: {e}");
                }
                return Ok(r > 0);
            }
        }

        fn fill(&mut self) -> Result<()> {
            let mut buf = vec![0u8; 64 * 1024];
            let mut ctl = vec![0u64; (CMSG_HDR + MAX_FDS * 4).div_ceil(8) * 2];
            let mut iov = IoVec { base: buf.as_mut_ptr() as *mut c_void, len: buf.len() };
            let mut h = MsgHdr {
                name: core::ptr::null_mut(),
                namelen: 0,
                iov: &mut iov,
                iovlen: 1,
                control: ctl.as_mut_ptr() as *mut c_void,
                controllen: ctl.len() * 8,
                flags: 0,
            };
            let n = loop {
                // SAFETY: the header points at live buffers.
                let n = unsafe { recvmsg(self.sock.as_raw_fd(), &mut h, MSG_CMSG_CLOEXEC) };
                if n < 0 {
                    let e = std::io::Error::last_os_error();
                    if e.kind() == std::io::ErrorKind::Interrupted {
                        continue;
                    }
                    return Err(self.fail(format!("read from the session bus: {e}")));
                }
                break n as usize;
            };
            if n == 0 {
                return Err(self.fail("the session bus closed the connection".into()));
            }
            let base = ctl.as_ptr() as *const u8;
            let mut off = 0;
            while off + CMSG_HDR <= h.controllen {
                // SAFETY: within the control buffer the kernel filled.
                let (len, level, ty) = unsafe {
                    ((base.add(off) as *const usize).read(), (base.add(off + 8) as *const c_int).read(), (base.add(off + 12) as *const c_int).read())
                };
                if len < CMSG_HDR || off + len > h.controllen {
                    break;
                }
                if level == SOL_SOCKET && ty == SCM_RIGHTS {
                    for i in 0..(len - CMSG_HDR) / 4 {
                        // SAFETY: a descriptor the kernel just gave us.
                        let fd = unsafe { (base.add(off + CMSG_HDR + i * 4) as *const c_int).read_unaligned() };
                        self.infds.push_back(unsafe { OwnedFd::from_raw_fd(fd) });
                    }
                }
                off += len.next_multiple_of(8);
            }
            if h.flags & MSG_CTRUNC != 0 {
                // Which message the lost descriptors belonged to is unknown:
                // the stream can no longer be trusted (those received close).
                return Err(self.fail("D-Bus: file descriptors were cut off".into()));
            }
            self.inbuf.extend_from_slice(&buf[..n]);
            Ok(())
        }

        /// The next message, waiting until `until`; `None` on timeout. A
        /// message that does not parse is dropped (its bytes and its
        /// descriptors) and is an error; a stream that lost its framing
        /// breaks the connection.
        pub(crate) fn read(&mut self, until: Instant) -> Result<Option<Message>> {
            self.check()?;
            loop {
                let n = match Message::wire_len(&self.inbuf) {
                    Err(e) => return Err(self.fail(format!("D-Bus stream: {e}"))),
                    Ok(n) => n,
                };
                if let Some(n) = n
                    && self.inbuf.len() >= n
                {
                    let got = Message::decode(&self.inbuf[..n]);
                    let nfd = match &got {
                        Ok((_, nfd)) => Some(*nfd),
                        Err(_) => Message::declared_fds(&self.inbuf[..n]),
                    };
                    self.inbuf.drain(..n);
                    let Some(nfd) = nfd else {
                        // Its descriptors cannot be told from the next ones'.
                        return Err(self.fail(format!("D-Bus: unreadable message header ({:#})", got.expect_err("an error"))));
                    };
                    let mut fds = Vec::new();
                    for _ in 0..nfd {
                        match self.infds.pop_front() {
                            Some(f) => fds.push(f),
                            None => return Err(self.fail("D-Bus: a message lacks its file descriptors".into())),
                        }
                    }
                    let (mut m, _) = got.map_err(|e| e.context("D-Bus: a message was dropped"))?; // `fds` close here
                    m.fds = fds.into_iter().map(Some).collect();
                    return Ok(Some(m));
                }
                if !self.readable(until)? {
                    return Ok(None);
                }
                self.fill()?;
            }
        }

        /// [`Conn::read`], passing over a message that was dropped (said on
        /// stderr): one bad message does not fail a call waiting for another.
        fn next_msg(&mut self, until: Instant) -> Result<Option<Message>> {
            loop {
                match self.read(until) {
                    Err(e) if self.broken.is_none() => eprintln!("rustshot: {e:#}"),
                    r => return r,
                }
            }
        }

        fn keep(&mut self, m: Message) {
            if self.kept.len() >= KEEP {
                self.kept.pop_front();
            }
            self.kept.push_back(m);
        }

        /// The next method call addressed to us (kept or arriving), waiting
        /// up to `timeout` (tests: a call to ourselves).
        #[allow(dead_code)] // tests; the StatusNotifierItem tray
        pub fn wait_call(&mut self, timeout: Duration) -> Result<Option<Message>> {
            if let Some(i) = self.kept.iter().position(|m| m.kind == Kind::Call) {
                return Ok(self.kept.remove(i));
            }
            let until = Instant::now() + timeout;
            while let Some(m) = self.next_msg(until)? {
                if m.kind == Kind::Call {
                    return Ok(Some(m));
                }
                self.keep(m);
            }
            Ok(None)
        }

        /// The next method call to us or signal (kept or arriving), in
        /// order, waiting up to `timeout` (the StatusNotifierItem tray's loop).
        pub fn wait_any(&mut self, timeout: Duration) -> Result<Option<Message>> {
            if let Some(m) = self.kept.pop_front() {
                return Ok(Some(m));
            }
            let until = Instant::now() + timeout;
            while let Some(m) = self.next_msg(until)? {
                if matches!(m.kind, Kind::Call | Kind::Signal) {
                    return Ok(Some(m));
                }
            }
            Ok(None)
        }
    }

    impl Bus for Conn {
        fn unique_name(&self) -> &str {
            &self.name
        }

        fn call(&mut self, m: Message, timeout: Duration) -> Result<Message> {
            let what = format!("{}.{}", m.iface.as_deref().unwrap_or(""), m.member.as_deref().unwrap_or(""));
            let serial = self.write(m)?;
            let until = Instant::now() + timeout;
            loop {
                let Some(r) = self.next_msg(until)? else { bail!("{what}: no reply within {} s", timeout.as_secs_f32()) };
                match r.kind {
                    Kind::Return if r.reply_to == Some(serial) => return Ok(r),
                    Kind::Error if r.reply_to == Some(serial) => {
                        let text = r.body.first().and_then(|v| v.as_str()).unwrap_or("").to_string();
                        return Err(anyhow::Error::new(DbusError { name: r.error.unwrap_or_default(), text }).context(what));
                    }
                    Kind::Signal | Kind::Call => self.keep(r),
                    _ => {} // a late reply to an earlier call
                }
            }
        }

        fn send(&mut self, m: Message) -> Result<u32> {
            self.write(m)
        }

        fn wait_signal(&mut self, want: &dyn Fn(&Message) -> bool, timeout: Duration) -> Result<Option<Message>> {
            if let Some(i) = self.kept.iter().position(|m| m.kind == Kind::Signal && want(m)) {
                return Ok(self.kept.remove(i));
            }
            let until = Instant::now() + timeout;
            while let Some(m) = self.next_msg(until)? {
                if m.kind == Kind::Signal && want(&m) {
                    return Ok(Some(m));
                }
                if matches!(m.kind, Kind::Signal | Kind::Call) {
                    self.keep(m);
                }
            }
            Ok(None)
        }
    }
}

#[cfg(test)]
pub(crate) mod tests;
