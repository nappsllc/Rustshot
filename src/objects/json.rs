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
//!
//! The Edit window's crop is `"crop":[x,y,w,h]` next to `"objects"`
//! ([`to_json_doc`]): an older build ignores it, so after the objects
//! come opaque `rect` fills over everything outside the crop
//! (`"stands_for":"crop"`), which it draws and this reader drops: what
//! a crop hides stays hidden in v0.2 and v0.3 too.
//!
//! The Edit window's image edits are `"transforms"` after the crop
//! ([`to_json_full`]), an ordered list applied on export after it:
//! `{"op":"flip","axis":"h"}`, `{"op":"rotate","deg":90}`,
//! `{"op":"resize","w":1920,"h":1080,"from":[3840,2160],"filter":"lanczos3"}`,
//! `{"op":"canvas","pad":[t,r,b,l],"bg":"#ffffff"|"presentation","radius":12,"shadow":true}`.
//! The objects stay in the original's pixels: an older build ignores the
//! key and shows the picture unturned, at its size, without the canvas,
//! with every object (every redaction) where it belongs on it, so nothing
//! needs a stand-in; turning, resizing and padding hide nothing. The one
//! exception is a filled, opaque callout of a turned picture: its outline
//! (rounded, cloud) in the unturned box is not quite this build's, so an
//! opaque `rect` under it over its whole box is written for older builds
//! (`"stands_for":"callout"`, dropped by this reader). This
//! reader fails closed on a stack it does not know (an unknown op, values
//! out of range, a turn after a resize).

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
    to_json_doc(objs, None, (0, 0))
}

/// What `annotations.json` holds: the objects, the Edit window's crop
/// (image px; `None`: the whole picture) and its image edits (applied
/// after the crop: [`crate::imgedit`]).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Doc {
    pub objects: Vec<Obj>,
    pub crop: Option<FRect>,
    pub edits: crate::imgedit::Edits,
}

/// The image edits as `"transforms"`: the orientation as a flip then a
/// rotate, then the resize, then the canvas (nothing when there are none).
fn transforms(out: &mut String, e: &crate::imgedit::Edits) {
    use crate::imgedit::Bg;
    if e.is_empty() {
        return;
    }
    out.push_str(",\"transforms\":[");
    let mut first = true;
    let mut op = |out: &mut String, name: &str| {
        out.push_str(if first { "{\"op\":" } else { ",{\"op\":" });
        first = false;
        string(out, name);
    };
    if e.orient.flip {
        op(out, "flip");
        W(out).str("axis", "h");
        out.push('}');
    }
    if !e.orient.rot.is_multiple_of(4) {
        op(out, "rotate");
        W(out).num("deg", (e.orient.rot % 4) as f32 * 90.0);
        out.push('}');
    }
    if let Some(r) = e.resize {
        op(out, "resize");
        let mut w = W(out);
        w.num("w", r.w as f32);
        w.num("h", r.h as f32);
        w.pt("from", Pt::new(r.from.0 as f32, r.from.1 as f32));
        w.str("filter", r.filter.id());
        out.push('}');
    }
    if let Some(c) = e.canvas {
        op(out, "canvas");
        let mut w = W(out);
        // `[top, right, bottom, left]` (CSS order), through the four-number
        // writer (its x, y, w, h are just the four slots here).
        w.rect("pad", FRect { x: c.top as f32, y: c.right as f32, w: c.bottom as f32, h: c.left as f32 });
        match c.bg {
            Bg::Solid(col) => w.color("bg", col),
            Bg::Presentation => w.str("bg", "presentation"),
        }
        w.num("radius", c.radius);
        w.bool("shadow", c.shadow);
        out.push('}');
    }
    out.push(']');
}

/// `"transforms"` read back (see [`transforms`]): rotates and flips in
/// any number and order, then at most one resize, then at most one
/// canvas. Anything else (an unknown op, a value out of range, a resize
/// before a turn) is an error: a newer build's stack is not guessed at.
fn transforms_of(v: Option<&V>) -> Result<crate::imgedit::Edits> {
    use crate::imgedit::{Bg, Edits, Filter, Orient, Pad, Resize, MAX_SIDE};
    let mut e = Edits::default();
    let items = match v {
        None | Some(V::Null) => return Ok(e),
        Some(V::Arr(a)) if a.len() <= 64 => a,
        Some(_) => bail!("annotations.json: transforms: not a list"),
    };
    // 0: turns, 1: after the resize, 2: after the canvas.
    let mut stage = 0;
    for it in items {
        let V::Obj(fields) = it else { bail!("annotations.json: a transform is not an object") };
        let f = F { ty: "transform", fields };
        let side = |k: &str| -> Result<u32> {
            let v = f.num(k)?;
            if v.fract() != 0.0 || !(1.0..=MAX_SIDE as f32).contains(&v) {
                bail!("annotations.json: transform.{k}: {v} is not a size");
            }
            Ok(v as u32)
        };
        match f.str("op")? {
            "rotate" | "flip" if stage > 0 => bail!("annotations.json: a turn after a resize or a canvas"),
            "rotate" => {
                let d = f.num("deg")?;
                let Some(q) = [90.0, 180.0, 270.0].iter().position(|v| *v == d).map(|i| i as u8 + 1) else {
                    bail!("annotations.json: rotate.deg: {d} is not 90, 180 or 270")
                };
                e.orient = e.orient.then(Orient { rot: q, flip: false });
            }
            "flip" => {
                e.orient = match f.str("axis")? {
                    "h" => e.orient.flipped_h(),
                    "v" => e.orient.flipped_v(),
                    a => bail!("annotations.json: flip.axis: {a:?}"),
                };
            }
            "resize" if stage > 0 => bail!("annotations.json: a second resize, or one after the canvas"),
            "resize" => {
                let [fw, fh] = f.nums::<2>("from")?;
                let from = |v: f32| if v.fract() == 0.0 && (1.0..=MAX_SIDE as f32).contains(&v) { Ok(v as u32) } else { Err(anyhow!("annotations.json: resize.from: {v}")) };
                let filter = Filter::from_id(f.str("filter")?).ok_or_else(|| anyhow!("annotations.json: resize.filter: unknown"))?;
                e.resize = Some(Resize { w: side("w")?, h: side("h")?, from: (from(fw)?, from(fh)?), filter });
                stage = 1;
            }
            "canvas" if stage > 1 => bail!("annotations.json: a second canvas"),
            "canvas" => {
                let [t, r, b, l] = f.nums::<4>("pad")?;
                let px = |v: f32| if v.fract() == 0.0 && (0.0..=crate::imgedit::frame::MAX_PAD as f32).contains(&v) { Ok(v as u32) } else { Err(anyhow!("annotations.json: canvas.pad: {v}")) };
                let bg = match f.need("bg")? {
                    V::Str(s) if s == "presentation" => Bg::Presentation,
                    _ => Bg::Solid(f.color("bg")?),
                };
                let radius = f.num("radius")?;
                if !(0.0..=crate::imgedit::frame::MAX_RADIUS).contains(&radius) {
                    bail!("annotations.json: canvas.radius: {radius}");
                }
                e.canvas = Some(Pad { top: px(t)?, right: px(r)?, bottom: px(b)?, left: px(l)?, bg, radius, shadow: f.bool_or("shadow", false)? });
                stage = 2;
            }
            o => bail!("annotations.json: unknown transform {o:?}"),
        }
    }
    // An output past the pixel cap (a resize with its canvas) fails closed.
    if let Some(r) = e.resize {
        let out = e.canvas.map_or((r.w, r.h), |c| c.size((r.w, r.h)));
        if !crate::imgedit::within_cap(out) {
            bail!("annotations.json: a resize to {} x {} is past {} pixels", out.0, out.1, crate::imgedit::MAX_PIXELS);
        }
    }
    Ok(e)
}

/// The color of a crop's stand-in fills.
const CROP_FILL: C4 = C4 { r: 0, g: 0, b: 0, a: 255 };

/// The stand-ins an older build draws for `crop` of a `size` picture:
/// opaque filled rects over everything outside it (overlapping at the
/// joins and past the picture's edges, so no rounded corner leaves a
/// pixel uncovered).
pub fn crop_stand_ins(crop: FRect, size: (u32, u32)) -> Vec<Obj> {
    let Some(c) = super::mask::crop_rect(crop, size) else { return Vec::new() };
    let (w, h) = (size.0 as f32, size.1 as f32);
    let g = 2.0;
    super::mask::outside(c, size)
        .into_iter()
        .map(|b| {
            let mut r = b;
            if b.y + b.h <= c.y + 0.5 || b.y >= c.y + c.h - 0.5 {
                // Top / bottom: across the picture and past its edges.
                r = FRect { x: -g, w: w + 2.0 * g, ..r };
                if b.y <= 0.5 {
                    r = FRect { y: -g, h: r.h + g, ..r };
                }
                if b.y + b.h >= h - 0.5 {
                    r.h += g;
                }
            } else {
                // Left / right: over the joins with the bands above and below.
                r = FRect { y: r.y - g, h: r.h + 2.0 * g, ..r };
                if b.x <= 0.5 {
                    r = FRect { x: -g, w: r.w + g, ..r };
                }
                if b.x + b.w >= w - 0.5 {
                    r.w += g;
                }
            }
            Obj::RectFill { r, color: CROP_FILL, width: 1.0 }
        })
        .collect()
}

/// Every stand-in for `crop` of a `size` picture with `objs`: the fills
/// over what the crop hides, and over the loupe of every magnifier that
/// reads from outside the crop (an older build would magnify the hidden
/// pixels into the visible part).
pub fn doc_stand_ins(objs: &[Obj], crop: FRect, size: (u32, u32)) -> Vec<Obj> {
    let mut v = crop_stand_ins(crop, size);
    let Some(c) = super::mask::crop_rect(crop, size) else { return v };
    for o in objs {
        if let Some((src, loupe)) = o.loupe_parts()
            && !(src.x >= c.x && src.y >= c.y && src.x + src.w <= c.x + c.w && src.y + src.h <= c.y + c.h)
        {
            v.push(Obj::RectFill { r: loupe, color: CROP_FILL, width: 1.0 });
        }
    }
    v
}

/// [`to_json`] with the Edit window's `crop` of a `size` picture: written
/// as `"crop":[x,y,w,h]` (this build) and, for older builds that know
/// nothing of it, opaque fills over what it hides (`"stands_for":"crop"`,
/// on top of every object), so a crop that hides something keeps it
/// hidden there too; this reader drops them.
pub fn to_json_doc(objs: &[Obj], crop: Option<FRect>, size: (u32, u32)) -> String {
    to_json_full(objs, crop, &Default::default(), size)
}

/// [`to_json_doc`] with the image edits as `"transforms"` after the crop.
/// The objects stay in the original's pixels, where an older build (that
/// ignores the key) draws them: it shows the picture unturned, at its
/// size, with every redaction where it belongs.
pub fn to_json_full(objs: &[Obj], crop: Option<FRect>, edits: &crate::imgedit::Edits, size: (u32, u32)) -> String {
    let mut out = String::new();
    let _ = write!(out, "{{\"version\":{WRITTEN}");
    let crop = crop.and_then(|c| super::mask::crop_rect(c, size)).filter(|c| *c != FRect { x: 0.0, y: 0.0, w: size.0 as f32, h: size.1 as f32 });
    if let Some(c) = crop {
        out.push_str(",\"crop\":");
        rect(&mut out, c);
    }
    transforms(&mut out, edits);
    out.push_str(",\"objects\":[");
    let stand_ins = crop.map_or(Vec::new(), |c| doc_stand_ins(objs, c, size));
    for (i, o) in objs.iter().chain(&stand_ins).enumerate() {
        out.push_str(if i == 0 { "\n " } else { ",\n " });
        if i >= objs.len() {
            if let Obj::RectFill { r, color, width } = o {
                out.push_str("{\"type\":\"rect\"");
                let mut w = W(&mut out);
                w.rect("r", *r);
                w.color("color", *color);
                w.num("width", *width);
                w.bool("fill", true);
                w.str("stands_for", "crop");
                out.push('}');
            }
            continue;
        }
        // A filled, opaque callout of a turned picture: an older build
        // draws it unturned, its rounded or cloud outline not quite where
        // this build's falls; an opaque rect under it over its whole box
        // (`"stands_for":"callout"`, dropped here) covers every pixel the
        // callout covers in this build's export.
        if let Obj::Callout { r, color, fill: true, .. } = o
            && color.a == 255
            && !edits.orient.is_identity()
        {
            out.push_str("{\"type\":\"rect\"");
            let mut w = W(&mut out);
            w.rect("r", *r);
            w.color("color", *color);
            w.num("width", 1.0);
            w.bool("fill", true);
            w.str("stands_for", "callout");
            out.push_str("},\n ");
        }
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
    from_json_doc(s).map(|d| d.objects)
}

/// [`from_json`] with the crop (`"crop"`; its stand-ins dropped). A crop
/// that is not four finite numbers is an error (fail closed).
pub fn from_json_doc(s: &str) -> Result<Doc> {
    let V::Obj(top) = parse(s)? else { bail!("annotations.json: not an object") };
    let crop = match top.iter().find(|(n, _)| n == "crop").map(|(_, v)| v) {
        None | Some(V::Null) => None,
        Some(_) => {
            let f = F { ty: "document", fields: &top };
            let r = f.rect("crop")?;
            if r.w <= 0.0 || r.h <= 0.0 {
                bail!("annotations.json: an empty crop");
            }
            Some(r)
        }
    };
    let objects = objects_of(&top)?;
    let edits = transforms_of(top.iter().find(|(n, _)| n == "transforms").map(|(_, v)| v))?;
    Ok(Doc { objects, crop, edits })
}

fn objects_of(top: &[(String, V)]) -> Result<Vec<Obj>> {
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
            // A crop's stand-ins (the crop itself is the document's).
            "rect" if matches!(f.get("stands_for"), Some(V::Str(s)) if s == "crop" || s == "callout") => continue,
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

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn every_kind() -> Vec<Obj> {
        let c = C4::rgb(240, 68, 56);
        let t = C4::new(250, 220, 0, 90);
        let r = FRect { x: 10.5, y: 20.25, w: 300.0, h: 0.1 };
        vec![
            Obj::Path { pts: vec![Pt::new(1.0, 2.0), Pt::new(3.5, -4.0), Pt::new(1e6, 7.125)], color: c, width: 3.0 },
            Obj::Line { a: Pt::new(0.0, 0.0), b: Pt::new(10.0, 10.0), color: c, width: 1.5 },
            Obj::Arrow { a: Pt::new(5.0, 6.0), b: Pt::new(70.0, 80.0), color: C4::new(1, 2, 3, 4), width: 4.0 },
            Obj::Rect { r, color: c, width: 3.0 },
            Obj::RectFill { r, color: c, width: 3.0 },
            Obj::Ellipse { r, color: c, width: 2.0 },
            Obj::EllipseFill { r, color: c, width: 2.0 },
            Obj::Marker { a: Pt::new(1.0, 1.0), b: Pt::new(2.0, 2.0), color: t, width: 15.0 },
            Obj::MarkerPath { pts: vec![Pt::new(1.0, 1.0), Pt::new(9.0, 3.0)], color: t, width: 15.0 },
            Obj::Text {
                pos: Pt::new(4.0, 5.0),
                text: "Quote \" back\\slash\nnew line\ttab · Привет 你好 \u{1F600} \u{1}".into(),
                color: c,
                size: 16.0,
                fmt: TextFmt::default(),
            },
            Obj::Pixelate { r, cell: 12.0 },
            Obj::Invert { r },
            Obj::Counter { c: Pt::new(9.0, 9.0), n: 3, color: c, d: 32.0, tail: None },
            Obj::Counter { c: Pt::new(9.0, 9.0), n: 4, color: c, d: 32.0, tail: Some(Pt::new(40.0, 50.0)) },
            Obj::Callout {
                r: FRect { x: 300.0, y: 40.0, w: 240.0, h: 96.0 },
                tail: Pt::new(320.0, 180.0),
                text: "Move this to the top,\nabove the summary".into(),
                color: c,
                size: 17.0,
                style: CalloutStyle::Cloud,
                fill: false,
                fmt: TextFmt::default(),
            },
            Obj::Spotlight { r: FRect { x: 1.0, y: 2.0, w: 3.0, h: 4.0 } },
            Obj::Magnify { src: Pt::new(100.0, 100.0), dst: Pt::new(250.0, 40.0), radius: 60.0, zoom: 2.0, color: C4::rgb(255, 255, 255), width: 3.0 },
        ]
    }

    /// Every kind of version 2: a blur, styled texts and callouts.
    pub(crate) fn every_v2_kind() -> Vec<Obj> {
        let c = C4::rgb(59, 130, 246);
        let all = TextFmt { family: Family::Serif, bold: true, italic: true, underline: true, strike: true, align: Align::Center };
        vec![
            Obj::Blur { r: FRect { x: 10.5, y: 20.0, w: 300.0, h: 120.0 }, sigma: 12.0 },
            Obj::Text { pos: Pt::new(4.0, 5.0), text: "Bold\nall".into(), color: c, size: 20.0, fmt: all },
            Obj::Text { pos: Pt::new(4.0, 5.0), text: "mono right".into(), color: c, size: 20.0, fmt: TextFmt { family: Family::Mono, align: Align::Right, ..Default::default() } },
            Obj::Text { pos: Pt::new(4.0, 5.0), text: "italic".into(), color: c, size: 20.0, fmt: TextFmt { italic: true, ..Default::default() } },
            Obj::Callout {
                r: FRect { x: 300.0, y: 40.0, w: 240.0, h: 96.0 },
                tail: Pt::new(320.0, 180.0),
                text: "Struck".into(),
                color: c,
                size: 17.0,
                style: CalloutStyle::Rounded,
                fill: true,
                // A callout has no alignment.
                fmt: TextFmt { align: Align::Left, ..all },
            },
        ]
    }

    #[test]
    fn round_trips_every_kind() {
        let objs = every_kind();
        let s = to_json(&objs);
        assert!(s.starts_with("{\"version\":1,"), "plain objects stay version 1: {s}");
        let back = from_json(&s).expect("parses");
        assert_eq!(back, objs);
        assert_eq!(to_json(&back), s, "stable");
        assert_eq!(from_json(&to_json(&[])).unwrap(), Vec::<Obj>::new());
        for o in every_v2_kind() {
            let one = vec![o];
            let s = to_json(&one);
            assert!(s.starts_with("{\"version\":1,"), "still version 1: {s}");
            assert_eq!(from_json(&s).expect("parses"), one, "{s}");
        }
        let mut all = every_kind();
        all.extend(every_v2_kind());
        let s = to_json(&all);
        assert_eq!(from_json(&s).unwrap(), all, "the stand-in is dropped");
        assert_eq!(to_json(&from_json(&s).unwrap()), s, "stable");
        assert!(s.contains("\"font\":\"serif\",\"bold\":true,\"italic\":true,\"underline\":true,\"strike\":true,\"align\":\"center\""), "{s}");
        assert!(
            s.contains("{\"type\":\"pixelate\",\"r\":[10.5,20,300,120],\"cell\":24,\"stands_for\":\"blur\"},\n {\"type\":\"blur\",\"r\":[10.5,20,300,120],\"sigma\":12}"),
            "{s}"
        );
        assert!(!to_json(&every_kind()).contains("\"bold\""), "defaults are not written");
    }

    /// What v0.2.0's own reader (`v020/json.rs`, its code byte for byte)
    /// makes of this build's documents: it reads them (version 1), draws
    /// the texts plain, and covers every blur with a pixelate of its rect
    /// at the blur's place in the stack; nothing else changes.
    #[test]
    fn v0_2_0_reads_the_documents_and_still_redacts() {
        use crate::objects::v020;
        let mut all = every_kind();
        all.extend(every_v2_kind());
        // A blur on top of a pixelate, and a styled text over a blur.
        all.push(Obj::Pixelate { r: FRect { x: 5.0, y: 5.0, w: 50.0, h: 50.0 }, cell: 8.0 });
        all.push(Obj::Blur { r: FRect { x: 0.0, y: 0.0, w: 80.0, h: 60.0 }, sigma: 30.0 });
        all.push(Obj::Text { pos: Pt::new(4.0, 5.0), text: "x".into(), color: C4::rgb(0, 0, 0), size: 12.0, fmt: TextFmt { bold: true, ..Default::default() } });
        let s = to_json(&all);
        let old = v020::json::from_json(&s).expect("v0.2.0 reads it");
        assert_eq!(old.len(), all.len(), "one object each (a blur's stand-in for the blur)");
        for (new, old) in all.iter().zip(&old) {
            match (new, old) {
                (Obj::Blur { r, sigma }, v020::Obj::Pixelate { r: q, cell }) => {
                    assert_eq!((r, *cell), (q, stand_in_cell(*sigma)), "the blur's rect is pixelated");
                    assert!(*cell >= 12.0);
                }
                (Obj::Blur { .. }, o) => panic!("a blur became {o:?}"),
                (Obj::Text { pos, text, color, size, .. }, v020::Obj::Text { pos: p, text: t, color: c, size: z }) => {
                    assert_eq!((pos, text, color, size), (p, t, c, z));
                }
                (Obj::Pixelate { r, cell }, v020::Obj::Pixelate { r: q, cell: c }) => assert_eq!((r, cell), (q, c)),
                (n, o) => assert_eq!(format!("{n:?}").split(' ').next(), format!("{o:?}").split(' ').next(), "same kind"),
            }
        }
    }

    /// A crop round-trips; an older build (v0.2.0's own reader) reads the
    /// document and draws opaque fills over every pixel outside it, on
    /// top of the objects, so nothing the crop hides shows there.
    #[test]
    fn crops_round_trip_and_stay_hidden_in_v0_2_0() {
        use crate::objects::v020;
        let size = (300, 200);
        let crop = FRect { x: 40.0, y: 30.0, w: 150.0, h: 100.0 };
        let mut objs = every_kind();
        objs.extend(every_v2_kind());
        let s = to_json_doc(&objs, Some(crop), size);
        assert!(s.starts_with("{\"version\":1,\"crop\":[40,30,150,100],\"objects\":["), "{s}");
        let d = from_json_doc(&s).unwrap();
        assert_eq!(d, Doc { objects: objs.clone(), crop: Some(crop), ..Default::default() }, "the stand-ins are dropped");
        assert_eq!(from_json(&s).unwrap(), objs);
        assert_eq!(to_json_doc(&d.objects, d.crop, size), s, "stable");
        // No crop (or the whole picture): as before.
        assert_eq!(to_json_doc(&objs, None, size), to_json(&objs));
        assert_eq!(to_json_doc(&objs, Some(FRect { x: 0.0, y: 0.0, w: 300.0, h: 200.0 }), size), to_json(&objs));
        assert_eq!(from_json_doc(&to_json(&objs)).unwrap().crop, None);
        // Fail closed on a broken crop.
        assert!(from_json_doc("{\"version\":1,\"crop\":[1,2],\"objects\":[]}").is_err());
        assert!(from_json_doc("{\"version\":1,\"crop\":[1,2,0,5],\"objects\":[]}").is_err());

        // v0.3.0's reader: every object as written, then the fills on top.
        let v3 = crate::objects::v030::json::from_json(&s).expect("v0.3.0 reads it");
        assert_eq!(&v3[..objs.len()], &objs[..]);
        assert_eq!(&v3[objs.len()..], &doc_stand_ins(&objs, crop, size)[..], "v0.3.0 draws the stand-ins");
        let old = v020::json::from_json(&s).expect("v0.2.0 reads it");
        let n = objs.len();
        assert_eq!(old.len(), n + doc_stand_ins(&objs, crop, size).len());
        // The crop's own fills (the loupe covers come after them).
        let fills: Vec<Obj> = old[n..n + crop_stand_ins(crop, size).len()]
            .iter()
            .map(|o| match o {
                v020::Obj::RectFill { r, color, width } => Obj::RectFill { r: *r, color: *color, width: *width },
                o => panic!("a stand-in became {o:?}"),
            })
            .collect();
        // Drawn as an older build draws them (same fill as this build's):
        // every pixel outside the crop is covered, opaque.
        let mut img = crate::pixbuf::PixBuf::from_pixel(size.0, size.1, [255, 255, 255, 255]);
        for f in &fills {
            f.render(&mut img, None);
        }
        for y in 0..size.1 {
            for x in 0..size.0 {
                let inside = (x as f32) >= crop.x + 1.0 && (x as f32) < crop.x + crop.w - 1.0 && (y as f32) >= crop.y + 1.0 && (y as f32) < crop.y + crop.h - 1.0;
                let p = img.get_pixel(x, y);
                if inside {
                    assert_eq!(p, [255, 255, 255, 255], "({x}, {y}) inside the crop");
                } else if !((x as f32) >= crop.x && (x as f32) < crop.x + crop.w && (y as f32) >= crop.y && (y as f32) < crop.y + crop.h) {
                    assert_eq!(p, [0, 0, 0, 255], "({x}, {y}) outside the crop shows");
                }
            }
        }
    }

    fn sample_edits() -> crate::imgedit::Edits {
        use crate::imgedit::{Bg, Edits, Filter, Orient, Pad, Resize};
        Edits {
            orient: Orient { rot: 1, flip: true },
            resize: Some(Resize { w: 50, h: 75, from: (100, 150), filter: Filter::Area }),
            canvas: Some(Pad { top: 1, right: 2, bottom: 3, left: 4, bg: Bg::Solid(C4::new(10, 20, 30, 40)), radius: 12.0, shadow: true }),
            ..Default::default()
        }
    }

    /// The image edits round-trip in every orientation; a stack this
    /// build does not know fails closed.
    #[test]
    fn transforms_round_trip_and_fail_closed() {
        use crate::imgedit::{Bg, Edits, Orient, Pad};
        let size = (150, 100);
        let objs = every_kind();
        let crop = Some(FRect { x: 10.0, y: 0.0, w: 100.0, h: 100.0 });
        let s = to_json_full(&objs, crop, &sample_edits(), size);
        assert!(
            s.starts_with(
                "{\"version\":1,\"crop\":[10,0,100,100],\"transforms\":[{\"op\":\"flip\",\"axis\":\"h\"},{\"op\":\"rotate\",\"deg\":90},\
                 {\"op\":\"resize\",\"w\":50,\"h\":75,\"from\":[100,150],\"filter\":\"area\"},\
                 {\"op\":\"canvas\",\"pad\":[1,2,3,4],\"bg\":\"#0a141e28\",\"radius\":12,\"shadow\":true}],\"objects\":["
            ),
            "{s}"
        );
        let d = from_json_doc(&s).unwrap();
        assert_eq!((d.edits, &d.objects, d.crop), (sample_edits(), &objs, crop));
        assert_eq!(to_json_full(&d.objects, d.crop, &d.edits, size), s, "stable");
        for i in 0..8 {
            let o = Orient { rot: i % 4, flip: i >= 4 };
            let canvas = (i % 2 == 0).then(Pad::presentation);
            let e = Edits { orient: o, canvas, ..Default::default() };
            let s = to_json_full(&[], None, &e, size);
            assert_eq!(from_json_doc(&s).unwrap().edits, e, "{s}");
            assert_eq!(s.contains("transforms"), !e.is_empty());
        }
        assert!(matches!(from_json_doc(&to_json_full(&[], None, &Edits { canvas: Some(Pad::presentation()), ..Default::default() }, size)).unwrap().edits.canvas, Some(Pad { bg: Bg::Presentation, .. })));
        // Any order of turns is read (composed); the rest must follow them.
        let turns = r##"{"version":1,"transforms":[{"op":"rotate","deg":90},{"op":"flip","axis":"v"},{"op":"rotate","deg":180}],"objects":[]}"##;
        assert_eq!(from_json_doc(turns).unwrap().edits.orient, Orient::ID.rotated(true).flipped_v().rotated(true).rotated(true));
        for bad in [
            r##"{"version":1,"transforms":{"op":"rotate"},"objects":[]}"##,
            r##"{"version":1,"transforms":[{"op":"rotate","deg":45}],"objects":[]}"##,
            r##"{"version":1,"transforms":[{"op":"skew","deg":5}],"objects":[]}"##,
            r##"{"version":1,"transforms":[{"op":"flip","axis":"d"}],"objects":[]}"##,
            r##"{"version":1,"transforms":[{"op":"resize","w":5,"h":5,"from":[10,10],"filter":"area"},{"op":"rotate","deg":90}],"objects":[]}"##,
            r##"{"version":1,"transforms":[{"op":"resize","w":5,"h":5,"from":[10,10],"filter":"magic"}],"objects":[]}"##,
            r##"{"version":1,"transforms":[{"op":"resize","w":0,"h":5,"from":[10,10],"filter":"area"}],"objects":[]}"##,
            // Past the pixel cap (200 MP): alone, and with its canvas.
            r##"{"version":1,"transforms":[{"op":"resize","w":20000,"h":20000,"from":[10,10],"filter":"area"}],"objects":[]}"##,
            r##"{"version":1,"transforms":[{"op":"resize","w":14000,"h":14000,"from":[10,10],"filter":"area"},{"op":"canvas","pad":[2000,2000,2000,2000],"bg":"#fff","radius":0}],"objects":[]}"##,
            r##"{"version":1,"transforms":[{"op":"resize","w":5.5,"h":5,"from":[10,10],"filter":"area"}],"objects":[]}"##,
            r##"{"version":1,"transforms":[{"op":"canvas","pad":[-1,0,0,0],"bg":"#fff","radius":0}],"objects":[]}"##,
            r##"{"version":1,"transforms":[{"op":"canvas","pad":[1,0,0,0],"bg":"wood","radius":0}],"objects":[]}"##,
            r##"{"version":1,"transforms":[{"op":"canvas","pad":[1,0,0,0],"bg":"#fff","radius":0},{"op":"canvas","pad":[1,0,0,0],"bg":"#fff","radius":0}],"objects":[]}"##,
            r##"{"version":1,"transforms":[{"op":"canvas","pad":[1,0,0,0],"bg":"#fff","radius":0},{"op":"flip","axis":"h"}],"objects":[]}"##,
        ] {
            assert!(from_json_doc(bad).is_err(), "{bad}");
        }
    }

    /// v0.2.0's and v0.3.0's own readers (and so this build before image
    /// edits, which reads the same) ignore `"transforms"`: they read the
    /// same objects as without them, in the original's pixels, and draw
    /// every redaction over what it hides. The new export shows nothing
    /// of the secrets either.
    #[test]
    fn older_builds_ignore_transforms_and_still_redact() {
        use crate::objects::{v020, v030};
        let size = (300u32, 200u32);
        // Secrets: red squares; each covered by an opaque fill.
        let mut pic = crate::pixbuf::PixBuf::from_pixel(size.0, size.1, [255, 255, 255, 255]);
        let secrets = [FRect { x: 30.0, y: 40.0, w: 40.0, h: 12.0 }, FRect { x: 150.0, y: 120.0, w: 60.0, h: 20.0 }, FRect { x: 260.0, y: 10.0, w: 20.0, h: 20.0 }];
        for s in &secrets {
            for y in s.y as u32..(s.y + s.h) as u32 {
                for x in s.x as u32..(s.x + s.w) as u32 {
                    let i = ((y * size.0 + x) * 4) as usize;
                    pic.as_raw_mut()[i..i + 4].copy_from_slice(&[230, 20, 20, 255]);
                }
            }
        }
        let black = C4::rgb(0, 0, 0);
        let mut objs: Vec<Obj> = secrets.iter().map(|s| Obj::RectFill { r: FRect { x: s.x - 3.0, y: s.y - 3.0, w: s.w + 6.0, h: s.h + 6.0 }, color: black, width: 1.0 }).collect();
        objs.push(Obj::Blur { r: FRect { x: 100.0, y: 20.0, w: 50.0, h: 40.0 }, sigma: 10.0 });
        objs.push(Obj::Arrow { a: Pt::new(10.0, 190.0), b: Pt::new(120.0, 150.0), color: C4::rgb(0, 120, 255), width: 4.0 });
        let crop = Some(FRect { x: 0.0, y: 0.0, w: 290.0, h: 200.0 });
        let red = |p: [u8; 4]| p[3] > 0 && p[0] > 150 && p[1] < 90 && p[2] < 90;
        let mut e = sample_edits();
        e.resize = None; // the full-size picture: every pixel is checked
        for edits in [e, sample_edits(), crate::imgedit::Edits { orient: crate::imgedit::Orient { rot: 2, flip: false }, ..Default::default() }] {
            let s = to_json_full(&objs, crop, &edits, size);
            let plain = to_json_doc(&objs, crop, size);
            // What they read: exactly what they read without the edits.
            let v3 = v030::json::from_json(&s).expect("v0.3.0 reads it");
            assert_eq!(v3, v030::json::from_json(&plain).unwrap());
            assert_eq!(&v3[..objs.len()], &objs[..], "in the original's pixels");
            let v2 = v020::json::from_json(&s).expect("v0.2.0 reads it");
            assert_eq!(format!("{v2:?}"), format!("{:?}", v020::json::from_json(&plain).unwrap()));
            for s in &secrets {
                assert!(
                    v2.iter().any(|o| matches!(o, v020::Obj::RectFill { r, .. } if r.x <= s.x && r.y <= s.y && r.x + r.w >= s.x + s.w && r.y + r.h >= s.y + s.h)),
                    "v0.2.0 covers {s:?}"
                );
            }
            // Drawn as v0.3 draws them (over the whole original): no secret shows.
            let old = crate::editor::bake_objects(&pic, &v3, None);
            assert!(!old.as_raw().as_chunks::<4>().0.iter().any(|p| red(*p)), "v0.3 shows a secret");
            // This build: turned, cropped, on its canvas, resized.
            let out = crate::imgedit::render(&pic, &objs, crop, None, &edits);
            assert_eq!(out.dimensions(), edits.out_size((290, 200)));
            assert!(!out.as_raw().as_chunks::<4>().0.iter().any(|p| red(*p)), "{edits:?}: the export shows a secret");
            // And without the fills, the secrets would show (the check bites).
            let bare = crate::imgedit::render(&pic, &objs[3..], crop, None, &edits);
            assert!(bare.as_raw().as_chunks::<4>().0.iter().any(|p| red(*p)));
        }
    }

    /// A filled callout drawn over a secret while the picture is turned
    /// (fractional box): this build keeps covering it after any turn and
    /// turn back; v0.2.0 and v0.3.0 draw the callout unturned with an
    /// opaque stand-in under it over its whole box, so the secret stays
    /// covered there too.
    #[test]
    fn a_filled_callout_drawn_turned_keeps_covering() {
        use crate::imgedit::{Edits, Orient};
        use crate::objects::{v020, v030};
        let size = (300u32, 200u32);
        // A tall secret (it reads across once the picture is turned).
        let secret = FRect { x: 120.0, y: 30.0, w: 24.0, h: 130.0 };
        let mut pic = crate::pixbuf::PixBuf::from_pixel(size.0, size.1, [255, 255, 255, 255]);
        for y in secret.y as u32..(secret.y + secret.h) as u32 {
            for x in secret.x as u32..(secret.x + secret.w) as u32 {
                let i = ((y * size.0 + x) * 4) as usize;
                pic.as_raw_mut()[i..i + 4].copy_from_slice(&[230, 20, 20, 255]);
            }
        }
        let red = |p: [u8; 4]| p[3] > 0 && p[0] > 150 && p[1] < 90 && p[2] < 90;
        let any_red = |img: &crate::pixbuf::PixBuf| img.as_raw().as_chunks::<4>().0.iter().any(|p| red(*p));
        for i in 1..8u8 {
            let o = Orient { rot: i % 4, flip: i >= 4 };
            // Drawn on the turned picture over the turned secret (a box a
            // little bigger, at half and quarter pixels).
            let s = o.rect(secret, size);
            let box_t = FRect { x: s.x - 12.5, y: s.y - 12.25, w: s.w + 25.0, h: s.h + 24.5 };
            let callout = Obj::Callout {
                r: box_t,
                tail: Pt::new(box_t.x + 10.0, box_t.y1() + 30.0),
                text: "covered".into(),
                color: C4::rgb(40, 40, 40),
                size: 14.0,
                style: CalloutStyle::Rounded,
                fill: true,
                fmt: TextFmt::default(),
            };
            let doc = o.inverse().map_obj(&callout, o.size(size), None);
            let Obj::Callout { r: d, .. } = &doc else { panic!() };
            assert_eq!(*d, o.inverse().rect(box_t, o.size(size)), "{o:?}: the box mapped exactly");
            // Turned and back in this build: the same box.
            assert_eq!(o.inverse().map_obj(&o.map_obj(&doc, size, None), o.size(size), None), doc, "{o:?}: turn, turn back");
            let edits = Edits { orient: o, ..Default::default() };
            let objs = vec![doc.clone()];
            assert!(!any_red(&crate::imgedit::render(&pic, &objs, None, None, &edits)), "{o:?}: this build shows the secret");
            for again in [Orient::ID.rotated(true), Orient::ID.flipped_h()] {
                let e2 = Edits { orient: o.then(again), ..Default::default() };
                assert!(!any_red(&crate::imgedit::render(&pic, &objs, None, None, &e2)), "{o:?} then {again:?}");
            }
            let js = to_json_full(&objs, None, &edits, size);
            assert!(js.contains("\"stands_for\":\"callout\""), "{js}");
            assert_eq!(from_json_doc(&js).unwrap().objects, objs, "the stand-in is dropped here");
            let v3 = v030::json::from_json(&js).unwrap();
            assert_eq!(v3.len(), 2, "the stand-in, then the callout");
            assert!(!any_red(&crate::editor::bake_objects(&pic, &v3, None)), "{o:?}: v0.3 shows the secret");
            let v2 = v020::json::from_json(&js).unwrap();
            assert!(matches!(&v2[0], v020::Obj::RectFill { r, color, .. } if *r == *d && color.a == 255), "v0.2.0 gets the stand-in under the callout");
            assert!(matches!(&v2[1], v020::Obj::Callout { .. }));
        }
        // Unturned, outlined or see-through: no stand-in.
        let c = Obj::Callout { r: secret, tail: Pt::new(0.0, 0.0), text: String::new(), color: C4::new(0, 0, 0, 120), size: 14.0, style: CalloutStyle::Rounded, fill: true, fmt: TextFmt::default() };
        let turned = Edits { orient: Orient::ID.rotated(true), ..Default::default() };
        assert!(!to_json_full(std::slice::from_ref(&c), None, &turned, size).contains("stands_for"));
        let mut opaque = c.clone();
        if let Obj::Callout { color, .. } = &mut opaque {
            *color = C4::rgb(0, 0, 0);
        }
        assert!(!to_json_full(std::slice::from_ref(&opaque), None, &Edits::default(), size).contains("stands_for"));
    }

    /// A magnifier inside the crop whose source lies outside it: this
    /// build blanks the hidden part before it magnifies; older builds
    /// (v0.2.0, v0.3.0 readers) get its loupe covered, so neither shows
    /// a pixel of what the crop hides.
    #[test]
    fn a_magnifier_never_shows_what_the_crop_hides() {
        let size = (300u32, 200u32);
        let crop = FRect { x: 100.0, y: 0.0, w: 200.0, h: 200.0 };
        // Red outside the crop (x < 100), white inside.
        let mut pic = crate::pixbuf::PixBuf::new(size.0, size.1);
        for (i, p) in pic.as_raw_mut().as_chunks_mut::<4>().0.iter_mut().enumerate() {
            *p = if i % size.0 as usize >= 100 { [255, 255, 255, 255] } else { [255, 0, 0, 255] };
        }
        let mag = Obj::Magnify { src: Pt::new(50.0, 100.0), dst: Pt::new(200.0, 100.0), radius: 40.0, zoom: 2.0, color: C4::rgb(0, 0, 255), width: 2.0 };
        let reddish = |p: [u8; 4]| p[3] > 0 && p[0] > 150 && p[1] < 100 && p[2] < 100;
        let in_loupe = |x: u32, y: u32| (x as f32 - 200.0).hypot(y as f32 - 100.0) < 36.0;
        // This build: the export.
        let out = crate::history::render(&pic, std::slice::from_ref(&mag), Some(crop), None);
        assert_eq!(out.dimensions(), (200, 200));
        for y in 0..200 {
            for x in 0..200 {
                assert!(!reddish(out.get_pixel(x, y)), "({x}, {y}) shows the hidden red");
            }
        }
        // Older builds: their reader, drawn over the whole picture.
        let s = to_json_doc(std::slice::from_ref(&mag), Some(crop), size);
        for old in [crate::objects::v030::json::from_json(&s).unwrap()] {
            let drawn = crate::editor::bake_objects(&pic, &old, None);
            for y in 0..200 {
                for x in 100..300 {
                    if in_loupe(x, y) {
                        assert!(!reddish(drawn.get_pixel(x, y)), "v0.3: ({x}, {y}) in the loupe shows the hidden red");
                    }
                }
            }
        }
        let v2 = crate::objects::v020::json::from_json(&s).unwrap();
        assert!(matches!(v2.last(), Some(crate::objects::v020::Obj::RectFill { .. })), "v0.2.0 gets the loupe's cover last");
        assert!(matches!(v2.first(), Some(crate::objects::v020::Obj::Magnify { .. })));
        // A magnifier reading inside the crop needs no cover.
        let inside = Obj::Magnify { src: Pt::new(150.0, 100.0), dst: Pt::new(240.0, 100.0), radius: 40.0, zoom: 2.0, color: C4::rgb(0, 0, 255), width: 2.0 };
        assert_eq!(doc_stand_ins(&[inside], crop, size).len(), crop_stand_ins(crop, size).len());
    }
    /// What v0.2.0 wrote (version 1, texts without style fields) reads
    /// as plain text; style fields with unknown names fall back.
    #[test]
    fn reads_version_1_files() {
        let v1 = r##"{"version":1,"objects":[
 {"type":"text","pos":[40,30],"text":"Hello","color":"#f04438","size":16},
 {"type":"callout","r":[300,40,240,96],"tail":[320,180],"text":"Hi","color":"#f04438","size":17,"style":"rounded","fill":true},
 {"type":"pixelate","r":[10,20,30,40],"cell":12}
]}
"##;
        let v = from_json(v1).unwrap();
        assert_eq!(v.len(), 3);
        assert!(matches!(&v[0], Obj::Text { fmt, .. } if fmt.is_plain()));
        assert!(matches!(&v[1], Obj::Callout { fmt, .. } if fmt.is_plain()));
        assert_eq!(to_json(&v), v1, "written back the same, still version 1");
        let odd = r##"{"version":2,"objects":[{"type":"text","pos":[0,0],"text":"x","color":"#000","size":12,"font":"fantasy","align":"justify","bold":true}]}"##;
        assert!(matches!(&from_json(odd).unwrap()[0], Obj::Text { fmt, .. } if *fmt == TextFmt { bold: true, ..Default::default() }));
        assert!(from_json(r##"{"version":2,"objects":[{"type":"text","pos":[0,0],"text":"x","color":"#000","size":12,"bold":1}]}"##).is_err());
        assert!(from_json("{\"version\":3,\"objects\":[]}").is_err(), "newer than this build");
    }

    #[test]
    fn reads_hand_written_json() {
        let s = r##" { "objects" : [ {"type":"arrow","width":4,"color":"#F04438","a":[10,20],"b":[2e2,8E1]},
            {"type":"sticker","x":1}, {"type":"text","pos":[0,0],"text":"\u00e9\ud83d\ude00\/","color":"#fff","size":12} ],
            "version": 1, "extra": {"nested": [null, true]} } "##;
        let v = from_json(s).unwrap();
        assert_eq!(v.len(), 2, "the unknown sticker is skipped");
        assert_eq!(v[0], Obj::Arrow { a: Pt::new(10.0, 20.0), b: Pt::new(200.0, 80.0), color: C4::rgb(240, 68, 56), width: 4.0 });
        assert!(matches!(&v[1], Obj::Text { text, color, .. } if text == "é😀/" && *color == C4::rgb(255, 255, 255)));
    }

    #[test]
    fn rejects_bad_documents() {
        for bad in [
            "",
            "[]",
            "{\"objects\":[]}",
            "{\"version\":3,\"objects\":[]}",
            "{\"version\":1}",
            "{\"version\":1,\"objects\":[{\"type\":\"line\",\"a\":[1,2]}]}",
            "{\"version\":1,\"objects\":[{\"type\":\"line\",\"a\":[1],\"b\":[1,2],\"color\":\"#000\",\"width\":1}]}",
            "{\"version\":1,\"objects\":[{\"type\":\"invert\",\"r\":[1,2,3,4]}]} trailing",
            "{\"version\":1,\"objects\":[{\"type\":\"text\",\"pos\":[0,0],\"text\":\"open,\"color\":\"#000\",\"size\":1}]}",
            "{\"version\":1,\"objects\":[{\"type\":\"rect\",\"r\":[0,0,1,1],\"color\":\"red\",\"width\":1}]}",
        ] {
            assert!(from_json(bad).is_err(), "{bad:?}");
        }
        let deep = "[".repeat(200);
        assert!(from_json(&deep).is_err());
    }
}
