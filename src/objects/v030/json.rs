//! `annotations.json`: the objects of a capture as a small versioned JSON
//! document, written and read by hand (no serde).
//!
//! ```json
//! {"version":1,"objects":[
//!  {"type":"arrow","a":[10,20],"b":[200,80],"color":"#f04438","width":4},
//!  {"type":"callout","r":[300,40,240,96],"tail":[320,180],"text":"Hi","color":"#f04438","size":17,"style":"rounded","fill":true}
//! ]}
//! ```
//!
//! Points are `[x, y]`, rects `[x, y, w, h]` (image pixels), colors
//! `#rrggbb` or `#rrggbbaa`. List order is z-order (first is bottom-most).
//! Types: `pencil` (`pts`, `color`, `width`), `line` / `arrow` (`a`, `b`,
//! `color`, `width`), `rect` / `ellipse` (`r`, `color`, `width`, `fill`),
//! `marker` (`a`/`b` straight or `pts` freehand, `color`, `width`), `text`
//! (`pos`, `text`, `color`, `size`), `pixelate` (`r`, `cell`), `invert`
//! (`r`), `number` (`c`, `n`, `color`, `d`, optional `tail`), `callout`
//! (`r`, `tail`, `text`, `color`, `size`, `style`, `fill`), `spotlight`
//! (`r`), `magnify` (`src`, `dst`, `radius`, `zoom`, `color`, `width`).
//! A reader skips types it does not know; a newer `version` is an error.
//!
//! This build adds, still as version 1 (an older reader looks fields up
//! by name and skips types it does not know, so it reads all of it):
//! `blur` (`r`, `sigma`) and the text style of `text` and `callout`:
//! `font` (`sans`, `serif`, `mono`), `bold`, `italic`, `underline`,
//! `strike` and (text) `align` (`left`, `center`, `right`), each written
//! only when not the default (sans, off, left). An older build draws the
//! text plain. A blur is written after a stand-in pixelate over the same
//! rect (`"stands_for":"blur"`): an older build skips the blur and draws
//! the pixelate, so what the blur hides stays hidden there too; this
//! reader drops the stand-in. (Checked against v0.2.0's own reader,
//! `v020/json.rs`.) Version 2 documents are read as well (none are
//! written).

use super::{CalloutStyle, FRect, Obj, Pt, TextFmt};
use crate::text::{Align, Family};
use crate::uifb::C4;
use anyhow::{anyhow, bail, Result};
use std::fmt::Write;

/// The newest format version this build reads.
pub const VERSION: u32 = 2;
/// The version it writes (what v0.2.0 reads).
pub const WRITTEN: u32 = 1;

/// The cell of a blur's stand-in pixelate: coarse, so an older build's
/// mosaic hides at least what the blur does.
pub fn stand_in_cell(sigma: f32) -> f32 {
    (sigma * 2.0).clamp(12.0, 100.0).round()
}
/// What a document may hold: objects, points of a stroke, characters of a
/// text (more is an error).
pub const MAX_OBJECTS: usize = 10_000;
pub const MAX_POINTS: usize = 100_000;
pub const MAX_TEXT: usize = 100_000;

fn num(out: &mut String, v: f32) {
    let v = if v.is_finite() { v } else { 0.0 };
    let _ = write!(out, "{v}");
}

fn pt(out: &mut String, p: Pt) {
    out.push('[');
    num(out, p.x);
    out.push(',');
    num(out, p.y);
    out.push(']');
}

fn rect(out: &mut String, r: FRect) {
    out.push('[');
    for (i, v) in [r.x, r.y, r.w, r.h].into_iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        num(out, v);
    }
    out.push(']');
}

pub(crate) fn string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn color(out: &mut String, c: C4) {
    let s = if c.a == 255 {
        format!("#{:02x}{:02x}{:02x}", c.r, c.g, c.b)
    } else {
        format!("#{:02x}{:02x}{:02x}{:02x}", c.r, c.g, c.b, c.a)
    };
    string(out, &s);
}

/// One object's fields after `"type"`.
struct W<'a>(&'a mut String);

impl W<'_> {
    fn key(&mut self, k: &str) -> &mut String {
        self.0.push(',');
        string(self.0, k);
        self.0.push(':');
        self.0
    }
    fn pt(&mut self, k: &str, p: Pt) {
        pt(self.key(k), p);
    }
    fn rect(&mut self, k: &str, r: FRect) {
        rect(self.key(k), r);
    }
    fn num(&mut self, k: &str, v: f32) {
        num(self.key(k), v);
    }
    fn color(&mut self, k: &str, c: C4) {
        color(self.key(k), c);
    }
    fn str(&mut self, k: &str, s: &str) {
        string(self.key(k), s);
    }
    fn bool(&mut self, k: &str, b: bool) {
        self.key(k).push_str(if b { "true" } else { "false" });
    }
    /// The text style's fields that are not the default (`align`: only
    /// for a text).
    fn fmt(&mut self, f: &TextFmt, align: bool) {
        if f.family != Family::Sans {
            self.str("font", f.family.id());
        }
        for (on, k) in [(f.bold, "bold"), (f.italic, "italic"), (f.underline, "underline"), (f.strike, "strike")] {
            if on {
                self.bool(k, true);
            }
        }
        if align && f.align != Align::Left {
            self.str("align", f.align.id());
        }
    }
    fn pts(&mut self, k: &str, ps: &[Pt]) {
        let out = self.key(k);
        out.push('[');
        for (i, p) in ps.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            pt(out, *p);
        }
        out.push(']');
    }
}

/// The objects as an `annotations.json` document (one object per line).
pub fn to_json(objs: &[Obj]) -> String {
    let mut out = String::new();
    let _ = write!(out, "{{\"version\":{WRITTEN},\"objects\":[");
    for (i, o) in objs.iter().enumerate() {
        out.push_str(if i == 0 { "\n " } else { ",\n " });
        if let Obj::Blur { r, sigma } = o {
            // The stand-in an older build draws (this one drops it).
            out.push_str("{\"type\":\"pixelate\"");
            let mut w = W(&mut out);
            w.rect("r", *r);
            w.num("cell", stand_in_cell(*sigma));
            w.str("stands_for", "blur");
            out.push_str("},\n ");
        }
        let ty = match o {
            Obj::Path { .. } => "pencil",
            Obj::Line { .. } => "line",
            Obj::Arrow { .. } => "arrow",
            Obj::Rect { .. } | Obj::RectFill { .. } => "rect",
            Obj::Ellipse { .. } | Obj::EllipseFill { .. } => "ellipse",
            Obj::Marker { .. } | Obj::MarkerPath { .. } => "marker",
            Obj::Text { .. } => "text",
            Obj::Pixelate { .. } => "pixelate",
            Obj::Invert { .. } => "invert",
            Obj::Counter { .. } => "number",
            Obj::Callout { .. } => "callout",
            Obj::Spotlight { .. } => "spotlight",
            Obj::Magnify { .. } => "magnify",
            Obj::Blur { .. } => "blur",
        };
        out.push_str("{\"type\":");
        string(&mut out, ty);
        let mut w = W(&mut out);
        match o {
            Obj::Path { pts, color, width } | Obj::MarkerPath { pts, color, width } => {
                w.pts("pts", pts);
                w.color("color", *color);
                w.num("width", *width);
            }
            Obj::Line { a, b, color, width } | Obj::Arrow { a, b, color, width } | Obj::Marker { a, b, color, width } => {
                w.pt("a", *a);
                w.pt("b", *b);
                w.color("color", *color);
                w.num("width", *width);
            }
            Obj::Rect { r, color, width } | Obj::Ellipse { r, color, width } => {
                w.rect("r", *r);
                w.color("color", *color);
                w.num("width", *width);
                w.bool("fill", false);
            }
            Obj::RectFill { r, color, width } | Obj::EllipseFill { r, color, width } => {
                w.rect("r", *r);
                w.color("color", *color);
                w.num("width", *width);
                w.bool("fill", true);
            }
            Obj::Text { pos, text, color, size, fmt } => {
                w.pt("pos", *pos);
                w.str("text", text);
                w.color("color", *color);
                w.num("size", *size);
                w.fmt(fmt, true);
            }
            Obj::Pixelate { r, cell } => {
                w.rect("r", *r);
                w.num("cell", *cell);
            }
            Obj::Blur { r, sigma } => {
                w.rect("r", *r);
                w.num("sigma", *sigma);
            }
            Obj::Invert { r } | Obj::Spotlight { r } => w.rect("r", *r),
            Obj::Counter { c, n, color, d, tail } => {
                w.pt("c", *c);
                w.num("n", *n as f32);
                w.color("color", *color);
                w.num("d", *d);
                if let Some(t) = tail {
                    w.pt("tail", *t);
                }
            }
            Obj::Callout { r, tail, text, color, size, style, fill, fmt } => {
                w.rect("r", *r);
                w.pt("tail", *tail);
                w.str("text", text);
                w.color("color", *color);
                w.num("size", *size);
                w.str("style", style.id());
                w.bool("fill", *fill);
                w.fmt(fmt, false);
            }
            Obj::Magnify { src, dst, radius, zoom, color, width } => {
                w.pt("src", *src);
                w.pt("dst", *dst);
                w.num("radius", *radius);
                w.num("zoom", *zoom);
                w.color("color", *color);
                w.num("width", *width);
            }
        }
        out.push('}');
    }
    out.push_str("\n]}\n");
    out
}

/// A parsed JSON value (also read by the history store's `meta.json`).
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum V {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<V>),
    Obj(Vec<(String, V)>),
}

struct P<'a> {
    s: &'a [u8],
    i: usize,
    depth: u32,
}

impl P<'_> {
    fn ws(&mut self) {
        while self.i < self.s.len() && matches!(self.s[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }

    fn err(&self, what: &str) -> anyhow::Error {
        anyhow!("annotations.json: {what} at byte {}", self.i)
    }

    fn eat(&mut self, c: u8) -> Result<()> {
        self.ws();
        if self.s.get(self.i) == Some(&c) {
            self.i += 1;
            Ok(())
        } else {
            Err(self.err(&format!("expected '{}'", c as char)))
        }
    }

    fn value(&mut self) -> Result<V> {
        self.ws();
        self.depth += 1;
        if self.depth > 64 {
            return Err(self.err("nested too deep"));
        }
        let v = match self.s.get(self.i) {
            Some(b'{') => {
                self.i += 1;
                let mut fields = Vec::new();
                self.ws();
                if self.s.get(self.i) == Some(&b'}') {
                    self.i += 1;
                } else {
                    loop {
                        self.ws();
                        let k = self.string()?;
                        self.eat(b':')?;
                        fields.push((k, self.value()?));
                        self.ws();
                        match self.s.get(self.i) {
                            Some(b',') => self.i += 1,
                            Some(b'}') => {
                                self.i += 1;
                                break;
                            }
                            _ => return Err(self.err("expected ',' or '}'")),
                        }
                    }
                }
                V::Obj(fields)
            }
            Some(b'[') => {
                self.i += 1;
                let mut items = Vec::new();
                self.ws();
                if self.s.get(self.i) == Some(&b']') {
                    self.i += 1;
                } else {
                    loop {
                        items.push(self.value()?);
                        self.ws();
                        match self.s.get(self.i) {
                            Some(b',') => self.i += 1,
                            Some(b']') => {
                                self.i += 1;
                                break;
                            }
                            _ => return Err(self.err("expected ',' or ']'")),
                        }
                    }
                }
                V::Arr(items)
            }
            Some(b'"') => V::Str(self.string()?),
            Some(b't') if self.s[self.i..].starts_with(b"true") => {
                self.i += 4;
                V::Bool(true)
            }
            Some(b'f') if self.s[self.i..].starts_with(b"false") => {
                self.i += 5;
                V::Bool(false)
            }
            Some(b'n') if self.s[self.i..].starts_with(b"null") => {
                self.i += 4;
                V::Null
            }
            Some(c) if *c == b'-' || c.is_ascii_digit() => {
                let a = self.i;
                self.i += 1;
                while self.i < self.s.len() && matches!(self.s[self.i], b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-') {
                    self.i += 1;
                }
                let t = std::str::from_utf8(&self.s[a..self.i]).map_err(|_| self.err("bad number"))?;
                V::Num(t.parse().map_err(|_| self.err("bad number"))?)
            }
            _ => return Err(self.err("expected a value")),
        };
        self.depth -= 1;
        Ok(v)
    }

    fn hex4(&mut self) -> Result<u32> {
        let h = self.s.get(self.i..self.i + 4).ok_or_else(|| self.err("short \\u escape"))?;
        let v = u32::from_str_radix(std::str::from_utf8(h).map_err(|_| self.err("bad \\u escape"))?, 16).map_err(|_| self.err("bad \\u escape"))?;
        self.i += 4;
        Ok(v)
    }

    fn string(&mut self) -> Result<String> {
        if self.s.get(self.i) != Some(&b'"') {
            return Err(self.err("expected a string"));
        }
        self.i += 1;
        let mut out = Vec::new();
        loop {
            let Some(&c) = self.s.get(self.i) else { return Err(self.err("unterminated string")) };
            self.i += 1;
            match c {
                b'"' => break,
                b'\\' => {
                    let Some(&e) = self.s.get(self.i) else { return Err(self.err("bad escape")) };
                    self.i += 1;
                    let ch = match e {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => {
                            let hi = self.hex4()?;
                            let cp = if (0xD800..0xDC00).contains(&hi) && self.s[self.i..].starts_with(b"\\u") {
                                // A pair; a high half without its low one
                                // is U+FFFD and the next escape stands.
                                let back = self.i;
                                self.i += 2;
                                let lo = self.hex4()?;
                                if (0xDC00..0xE000).contains(&lo) {
                                    0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00)
                                } else {
                                    self.i = back;
                                    0xFFFD
                                }
                            } else {
                                hi
                            };
                            char::from_u32(cp).unwrap_or('\u{FFFD}')
                        }
                        _ => return Err(self.err("bad escape")),
                    };
                    let mut b = [0u8; 4];
                    out.extend_from_slice(ch.encode_utf8(&mut b).as_bytes());
                }
                c => out.push(c),
            }
        }
        String::from_utf8(out).map_err(|_| self.err("invalid UTF-8"))
    }
}

pub(crate) fn parse(s: &str) -> Result<V> {
    let mut p = P { s: s.as_bytes(), i: 0, depth: 0 };
    let v = p.value()?;
    p.ws();
    if p.i != p.s.len() {
        return Err(p.err("trailing data"));
    }
    Ok(v)
}

/// Fields of one object, with typed getters.
struct F<'a> {
    ty: &'a str,
    fields: &'a [(String, V)],
}

impl F<'_> {
    fn get(&self, k: &str) -> Option<&V> {
        self.fields.iter().find(|(n, _)| n == k).map(|(_, v)| v)
    }
    fn need(&self, k: &str) -> Result<&V> {
        self.get(k).ok_or_else(|| anyhow!("annotations.json: {} without \"{k}\"", self.ty))
    }
    fn nums<const N: usize>(&self, k: &str) -> Result<[f32; N]> {
        match self.need(k)? {
            V::Arr(a) if a.len() == N => {
                let mut out = [0.0; N];
                for (o, v) in out.iter_mut().zip(a) {
                    let V::Num(n) = v else { bail!("annotations.json: {}.{k}: not a number", self.ty) };
                    *o = self.finite(k, *n)?;
                }
                Ok(out)
            }
            _ => bail!("annotations.json: {}.{k}: expected {N} numbers", self.ty),
        }
    }
    /// A finite number (as f32: out of its range is not finite either).
    fn finite(&self, k: &str, v: f64) -> Result<f32> {
        let f = v as f32;
        if !f.is_finite() {
            bail!("annotations.json: {}.{k}: {v} is not a finite number", self.ty);
        }
        Ok(f)
    }
    fn pt(&self, k: &str) -> Result<Pt> {
        let [x, y] = self.nums::<2>(k)?;
        Ok(Pt::new(x, y))
    }
    fn rect(&self, k: &str) -> Result<FRect> {
        let [x, y, w, h] = self.nums::<4>(k)?;
        Ok(FRect { x, y, w, h })
    }
    fn num(&self, k: &str) -> Result<f32> {
        match self.need(k)? {
            V::Num(n) => self.finite(k, *n),
            _ => bail!("annotations.json: {}.{k}: not a number", self.ty),
        }
    }
    fn str(&self, k: &str) -> Result<&str> {
        match self.need(k)? {
            V::Str(s) if s.chars().count() > MAX_TEXT => bail!("annotations.json: {}.{k}: longer than {MAX_TEXT} characters", self.ty),
            V::Str(s) => Ok(s),
            _ => bail!("annotations.json: {}.{k}: not a string", self.ty),
        }
    }
    fn bool_or(&self, k: &str, d: bool) -> Result<bool> {
        match self.get(k) {
            None => Ok(d),
            Some(V::Bool(b)) => Ok(*b),
            Some(_) => bail!("annotations.json: {}.{k}: not true/false", self.ty),
        }
    }
    /// The text style (each field optional: a missing one is the
    /// default; an unknown font or alignment name too, from a newer
    /// build).
    fn fmt(&self, align: bool) -> Result<TextFmt> {
        let name = |k: &str| -> Result<Option<&str>> { if self.get(k).is_some() { self.str(k).map(Some) } else { Ok(None) } };
        Ok(TextFmt {
            family: name("font")?.and_then(Family::from_id).unwrap_or_default(),
            bold: self.bool_or("bold", false)?,
            italic: self.bool_or("italic", false)?,
            underline: self.bool_or("underline", false)?,
            strike: self.bool_or("strike", false)?,
            align: if align { name("align")?.and_then(Align::from_id).unwrap_or_default() } else { Align::Left },
        })
    }
    fn color(&self, k: &str) -> Result<C4> {
        let s = self.str(k)?;
        let (r, g, b, a) = crate::config::parse_color(s).ok_or_else(|| anyhow!("annotations.json: {}.{k}: bad color {s:?}", self.ty))?;
        Ok(C4::new(r, g, b, a))
    }
    fn pts(&self, k: &str) -> Result<Vec<Pt>> {
        let V::Arr(a) = self.need(k)? else { bail!("annotations.json: {}.{k}: expected points", self.ty) };
        if a.len() > MAX_POINTS {
            bail!("annotations.json: {}.{k}: more than {MAX_POINTS} points", self.ty);
        }
        a.iter()
            .map(|p| match p {
                V::Arr(xy) if xy.len() == 2 => match (&xy[0], &xy[1]) {
                    (V::Num(x), V::Num(y)) => Ok(Pt::new(self.finite(k, *x)?, self.finite(k, *y)?)),
                    _ => bail!("annotations.json: {}.{k}: bad point", self.ty),
                },
                _ => bail!("annotations.json: {}.{k}: bad point", self.ty),
            })
            .collect()
    }
}

/// The objects of an `annotations.json` document. Unknown object types
/// are skipped; a malformed document, a missing field, a number that is
/// not finite, more than [`MAX_OBJECTS`] objects or a `version` newer
/// than [`VERSION`] is an error. What is read is made safe to render
/// ([`super::edit::sanitize`]: sizes in range, rects normalised, points
/// within a million pixels).
pub fn from_json(s: &str) -> Result<Vec<Obj>> {
    let V::Obj(top) = parse(s)? else { bail!("annotations.json: not an object") };
    let get = |k: &str| top.iter().find(|(n, _)| n == k).map(|(_, v)| v);
    match get("version") {
        Some(V::Num(v)) if *v >= 1.0 && *v <= VERSION as f64 && v.fract() == 0.0 => {}
        Some(V::Num(v)) => bail!("annotations.json: version {v} is not supported (this build reads up to {VERSION})"),
        Some(_) => bail!("annotations.json: the version is not a number"),
        _ => bail!("annotations.json: no version"),
    }
    let Some(V::Arr(items)) = get("objects") else { bail!("annotations.json: no objects list") };
    if items.len() > MAX_OBJECTS {
        bail!("annotations.json: more than {MAX_OBJECTS} objects");
    }
    let mut out = Vec::with_capacity(items.len());
    for it in items {
        let V::Obj(fields) = it else { bail!("annotations.json: an object is not an object") };
        let ty = match fields.iter().find(|(n, _)| n == "type") {
            Some((_, V::Str(t))) => t.as_str(),
            _ => bail!("annotations.json: an object without a type"),
        };
        let f = F { ty, fields };
        let o = match ty {
            "pencil" => Obj::Path { pts: f.pts("pts")?, color: f.color("color")?, width: f.num("width")? },
            "line" => Obj::Line { a: f.pt("a")?, b: f.pt("b")?, color: f.color("color")?, width: f.num("width")? },
            "arrow" => Obj::Arrow { a: f.pt("a")?, b: f.pt("b")?, color: f.color("color")?, width: f.num("width")? },
            "rect" | "ellipse" => {
                let (r, color, width) = (f.rect("r")?, f.color("color")?, f.num("width")?);
                match (ty, f.bool_or("fill", false)?) {
                    ("rect", false) => Obj::Rect { r, color, width },
                    ("rect", true) => Obj::RectFill { r, color, width },
                    (_, false) => Obj::Ellipse { r, color, width },
                    (_, true) => Obj::EllipseFill { r, color, width },
                }
            }
            "marker" if f.get("pts").is_some() => Obj::MarkerPath { pts: f.pts("pts")?, color: f.color("color")?, width: f.num("width")? },
            "marker" => Obj::Marker { a: f.pt("a")?, b: f.pt("b")?, color: f.color("color")?, width: f.num("width")? },
            "text" => Obj::Text {
                pos: f.pt("pos")?,
                text: f.str("text")?.to_string(),
                color: f.color("color")?,
                size: f.num("size")?,
                fmt: f.fmt(true)?,
            },
            // A blur's stand-in for older builds: the blur follows it.
            "pixelate" if matches!(f.get("stands_for"), Some(V::Str(s)) if s == "blur") => continue,
            "pixelate" => Obj::Pixelate { r: f.rect("r")?, cell: f.num("cell")? },
            "blur" => Obj::Blur { r: f.rect("r")?, sigma: f.num("sigma")? },
            "invert" => Obj::Invert { r: f.rect("r")? },
            "spotlight" => Obj::Spotlight { r: f.rect("r")? },
            "number" => Obj::Counter {
                c: f.pt("c")?,
                n: match f.need("n")? {
                    V::Num(n) if n.is_finite() => n.clamp(0.0, 1.0e6).round() as u32,
                    _ => bail!("annotations.json: number.n: not a finite number"),
                },
                color: f.color("color")?,
                d: f.num("d")?,
                tail: if f.get("tail").is_some() { Some(f.pt("tail")?) } else { None },
            },
            "callout" => Obj::Callout {
                r: f.rect("r")?,
                tail: f.pt("tail")?,
                text: f.str("text")?.to_string(),
                color: f.color("color")?,
                size: f.num("size")?,
                style: CalloutStyle::from_id(f.str("style")?).unwrap_or_default(),
                fill: f.bool_or("fill", true)?,
                fmt: f.fmt(false)?,
            },
            "magnify" => Obj::Magnify {
                src: f.pt("src")?,
                dst: f.pt("dst")?,
                radius: f.num("radius")?,
                zoom: f.num("zoom")?,
                color: f.color("color")?,
                width: f.num("width")?,
            },
            _ => continue,
        };
        let mut o = o;
        super::edit::sanitize(&mut o);
        out.push(o);
    }
    Ok(out)
}
