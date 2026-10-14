//! The image effects in annotations.json: a top-level `"effects"` list
//! after `"transforms"` (a key older builds ignore: effects add to the
//! picture and hide nothing, so they need no stand-in):
//! `{"op":"watermark","text":"…"|"image":"C:\\…\\logo.png","anchor":"bottom-right","margin":24,"opacity":0.6,"size":20,"tiled":false,"color":"#ffffff"}`,
//! `{"op":"torn","sides":"bottom,right","depth":12,"tooth":18,"seed":1}`,
//! `{"op":"shadow","offset":[0,8],"blur":24,"color":"#000000","opacity":0.25}`,
//! `{"op":"reflection","height":30,"opacity":0.4,"gap":0}`.
//! An op this build does not know (a newer build's) is skipped; a known
//! one with a value out of range is an error.

use super::{F, Pt, V, W, string};
use crate::imgedit::effects::{self, Effects, Reflection, Shadow, Str, Torn, Watermark};
use anyhow::{Result, bail};

const SIDES: [(u8, &str); 4] = [(effects::TOP, "top"), (effects::RIGHT, "right"), (effects::BOTTOM, "bottom"), (effects::LEFT, "left")];

pub(super) fn write(out: &mut String, e: &Effects) {
    if e.is_empty() {
        return;
    }
    let e = e.sanitized();
    out.push_str(",\"effects\":[");
    let mut first = true;
    let mut op = |out: &mut String, name: &str| {
        out.push_str(if first { "{\"op\":" } else { ",{\"op\":" });
        first = false;
        string(out, name);
    };
    if let Some(m) = e.watermark.filter(|m| !m.text.is_empty()) {
        op(out, "watermark");
        let mut w = W(out);
        w.str(if m.image { "image" } else { "text" }, m.text.as_str());
        w.str("anchor", effects::ANCHORS[m.anchor as usize]);
        w.num("margin", m.margin as f32);
        w.num("opacity", m.opacity);
        w.num("size", m.size);
        w.bool("tiled", m.tiled);
        w.color("color", m.color);
        out.push('}');
    }
    if let Some(t) = e.torn.filter(|t| t.sides != 0) {
        op(out, "torn");
        let mut w = W(out);
        let sides: Vec<&str> = SIDES.iter().filter(|(b, _)| t.sides & b != 0).map(|(_, n)| *n).collect();
        w.str("sides", &sides.join(","));
        w.num("depth", t.depth as f32);
        w.num("tooth", t.tooth as f32);
        w.num("seed", t.seed as f32);
        out.push('}');
    }
    if let Some(s) = e.shadow {
        op(out, "shadow");
        let mut w = W(out);
        w.pt("offset", Pt::new(s.dx as f32, s.dy as f32));
        w.num("blur", s.blur);
        w.color("color", s.color);
        w.num("opacity", s.opacity);
        out.push('}');
    }
    if let Some(r) = e.reflection {
        op(out, "reflection");
        let mut w = W(out);
        w.num("height", r.height);
        w.num("opacity", r.opacity);
        w.num("gap", r.gap as f32);
        out.push('}');
    }
    out.push(']');
}

fn ranged(f: &F, k: &str, lo: f32, hi: f32) -> Result<f32> {
    let v = f.num(k)?;
    if !(lo..=hi).contains(&v) {
        bail!("annotations.json: {}.{k}: {v} is out of range", f.ty);
    }
    Ok(v)
}

fn whole(f: &F, k: &str, lo: f32, hi: f32) -> Result<u32> {
    let v = ranged(f, k, lo, hi)?;
    if v.fract() != 0.0 {
        bail!("annotations.json: {}.{k}: {v} is not whole", f.ty);
    }
    Ok(v as u32)
}

pub(super) fn read(v: Option<&V>) -> Result<Effects> {
    let mut e = Effects::default();
    let items = match v {
        None | Some(V::Null) => return Ok(e),
        Some(V::Arr(a)) if a.len() <= 64 => a,
        Some(_) => bail!("annotations.json: effects: not a list"),
    };
    for it in items {
        let V::Obj(fields) = it else { bail!("annotations.json: an effect is not an object") };
        let probe = F { ty: "effect", fields };
        let op = probe.str("op")?;
        let f = F { ty: op, fields };
        match op {
            "watermark" => {
                let (image, text) = match (f.get("text"), f.get("image")) {
                    (Some(_), None) => (false, f.str("text")?),
                    (None, Some(_)) => (true, f.str("image")?),
                    _ => bail!("annotations.json: watermark: one of text and image"),
                };
                if text.len() > effects::MAX_WM {
                    bail!("annotations.json: watermark: longer than {} bytes", effects::MAX_WM);
                }
                let anchor = f.str("anchor")?;
                let Some(anchor) = effects::ANCHORS.iter().position(|a| *a == anchor) else {
                    bail!("annotations.json: watermark.anchor: {anchor:?}")
                };
                e.watermark = Some(Watermark {
                    image,
                    text: Str::new(text),
                    anchor: anchor as u8,
                    margin: whole(&f, "margin", 0.0, effects::MAX_MARGIN as f32)?,
                    opacity: ranged(&f, "opacity", 0.0, 1.0)?,
                    size: ranged(&f, "size", 1.0, 100.0)?,
                    tiled: f.bool_or("tiled", false)?,
                    color: f.color("color")?,
                });
            }
            "torn" => {
                let mut sides = 0;
                for s in f.str("sides")?.split(',').filter(|s| !s.is_empty()) {
                    let Some((b, _)) = SIDES.iter().find(|(_, n)| *n == s) else { bail!("annotations.json: torn.sides: {s:?}") };
                    sides |= b;
                }
                e.torn = Some(Torn {
                    sides,
                    depth: whole(&f, "depth", 1.0, effects::MAX_DEPTH as f32)?,
                    tooth: whole(&f, "tooth", 2.0, effects::MAX_TOOTH as f32)?,
                    seed: whole(&f, "seed", 0.0, 16_777_216.0)?,
                });
            }
            "shadow" => {
                let o = f.pt("offset")?;
                let m = effects::MAX_OFFSET as f32;
                if o.x.fract() != 0.0 || o.y.fract() != 0.0 || o.x.abs() > m || o.y.abs() > m {
                    bail!("annotations.json: shadow.offset: out of range");
                }
                e.shadow = Some(Shadow {
                    dx: o.x as i32,
                    dy: o.y as i32,
                    blur: ranged(&f, "blur", 0.0, effects::MAX_BLUR)?,
                    color: f.color("color")?,
                    opacity: ranged(&f, "opacity", 0.0, 1.0)?,
                });
            }
            "reflection" => {
                e.reflection = Some(Reflection {
                    height: ranged(&f, "height", 1.0, 100.0)?,
                    opacity: ranged(&f, "opacity", 0.0, 1.0)?,
                    gap: whole(&f, "gap", 0.0, effects::MAX_MARGIN as f32)?,
                });
            }
            _ => continue,
        }
    }
    Ok(e)
}

#[cfg(test)]
mod tests {
    use super::super::{from_json_doc, to_json_full};
    use super::*;
    use crate::imgedit::Edits;
    use crate::uifb::C4;

    fn all() -> Effects {
        Effects {
            watermark: Some(Watermark { text: Str::new("© Rustshot \"q\""), anchor: 2, margin: 7, opacity: 0.5, size: 33.0, tiled: true, color: C4::new(1, 2, 3, 200), image: false }),
            torn: Some(Torn { sides: effects::TOP | effects::LEFT, depth: 9, tooth: 21, seed: 77 }),
            shadow: Some(Shadow { dx: -3, dy: 11, blur: 30.0, color: C4::rgb(10, 20, 30), opacity: 0.75 }),
            reflection: Some(Reflection { height: 45.0, opacity: 0.25, gap: 4 }),
        }
    }

    #[test]
    fn effects_round_trip() {
        let e = Edits { effects: all(), ..Default::default() };
        let s = to_json_full(&[], None, &e, (100, 100));
        assert!(s.contains("\"effects\":["));
        assert_eq!(from_json_doc(&s).unwrap().edits.effects, all());
        let img = Edits { effects: Effects { watermark: Some(Watermark { image: true, text: Str::new("C:\\a b\\logo.png"), ..Default::default() }), ..Default::default() }, ..Default::default() };
        let s = to_json_full(&[], None, &img, (10, 10));
        assert_eq!(from_json_doc(&s).unwrap().edits, img);
        // none: no key
        assert!(!to_json_full(&[], None, &Edits::default(), (10, 10)).contains("effects"));
    }

    #[test]
    fn unknown_effects_are_skipped_and_bad_ones_fail() {
        let doc = |fx: &str| format!("{{\"version\":1,\"objects\":[],\"effects\":[{fx}]}}");
        let e = from_json_doc(&doc("{\"op\":\"sparkles\",\"n\":3},{\"op\":\"reflection\",\"height\":10,\"opacity\":0.5,\"gap\":0}")).unwrap();
        assert_eq!(e.edits.effects.reflection.map(|r| r.height), Some(10.0));
        for bad in [
            "{\"op\":\"reflection\",\"height\":0,\"opacity\":0.5,\"gap\":0}",
            "{\"op\":\"shadow\",\"offset\":[0,9999],\"blur\":1,\"color\":\"#000000\",\"opacity\":1}",
            "{\"op\":\"torn\",\"sides\":\"middle\",\"depth\":3,\"tooth\":3,\"seed\":0}",
            "{\"op\":\"watermark\",\"text\":\"a\",\"image\":\"b\",\"anchor\":\"top\",\"margin\":0,\"opacity\":1,\"size\":5,\"color\":\"#fff\"}",
            "{\"op\":\"watermark\",\"text\":\"a\",\"anchor\":\"nowhere\",\"margin\":0,\"opacity\":1,\"size\":5,\"color\":\"#ffffff\"}",
            "3",
        ] {
            assert!(from_json_doc(&doc(bad)).is_err(), "{bad}");
        }
    }

    /// The pointer object round-trips (its picture as a PNG); older
    /// builds skip the kind (it hides nothing); a big or broken picture
    /// fails closed.
    #[test]
    fn the_pointer_round_trips_and_older_builds_skip_it() {
        use crate::objects::{Obj, Pt};
        let mut img = crate::pixbuf::PixBuf::new(3, 4);
        img.as_raw_mut()[..8].copy_from_slice(&[1, 2, 3, 255, 9, 8, 7, 100]);
        let c = Obj::Cursor { at: Pt::new(10.0, 20.5), img: std::sync::Arc::new(img) };
        let line = Obj::Line { a: Pt::new(0.0, 0.0), b: Pt::new(5.0, 5.0), color: C4::rgb(1, 2, 3), width: 2.0 };
        let s = to_json_full(&[line.clone(), c.clone()], None, &Edits::default(), (100, 100));
        assert_eq!(from_json_doc(&s).unwrap().objects, vec![line.clone(), c]);
        assert_eq!(crate::objects::v020::json::from_json(&s).unwrap().len(), 1, "v0.2.0: the line only");
        assert_eq!(crate::objects::v030::json::from_json(&s).unwrap(), vec![line]);
        let bad = "{\"version\":1,\"objects\":[{\"type\":\"cursor\",\"at\":[0,0],\"png\":\"bm90IGEgcG5n\"}]}";
        assert!(from_json_doc(bad).is_err());
        let big = crate::pixbuf::PixBuf::new(300, 2).to_png().unwrap();
        let doc = format!("{{\"version\":1,\"objects\":[{{\"type\":\"cursor\",\"at\":[0,0],\"png\":\"{}\"}}]}}", crate::uploads::b64_encode(&big));
        assert!(from_json_doc(&doc).is_err(), "past 256 px");
    }

    /// Older builds read the document (v0.3's reader and v0.2's ignore the key).
    #[test]
    fn older_builds_ignore_effects() {
        let e = Edits { effects: all(), ..Default::default() };
        let s = to_json_full(&[], None, &e, (100, 100));
        assert!(crate::objects::v020::json::from_json(&s).is_ok());
        assert!(crate::objects::v030::json::from_json(&s).is_ok());
    }
}
