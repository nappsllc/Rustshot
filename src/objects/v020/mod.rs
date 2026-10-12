//! Tests only: the annotations.json reader of v0.2.0, to check what an
//! older build makes of the documents this one writes. `json.rs` is
//! `git show v0.2.0:src/objects/json.rs`, byte for byte; around it, the
//! v0.2.0 object enum (copied from v0.2.0's `objects/mod.rs`) and the
//! types it shares with this build (`Pt`, `FRect`, the callout style).
//! v0.2.0's `edit::sanitize` only clamps sizes and coordinates, so it is
//! left out (identity) here.

#![allow(dead_code)]

pub use super::{CalloutStyle, FRect, Pt};
use crate::uifb::C4;

pub mod json;

pub mod edit {
    pub fn sanitize(_o: &mut super::Obj) {}
}

#[derive(Clone, Debug, PartialEq)]
pub enum Obj {
    Line { a: Pt, b: Pt, color: C4, width: f32 },
    Arrow { a: Pt, b: Pt, color: C4, width: f32 },
    Rect { r: FRect, color: C4, width: f32 },
    Ellipse { r: FRect, color: C4, width: f32 },
    Path { pts: Vec<Pt>, color: C4, width: f32 },
    Marker { a: Pt, b: Pt, color: C4, width: f32 },
    Text { pos: Pt, text: String, color: C4, size: f32 },
    Pixelate { r: FRect, cell: f32 },
    Invert { r: FRect },
    MarkerPath { pts: Vec<Pt>, color: C4, width: f32 },
    RectFill { r: FRect, color: C4, width: f32 },
    EllipseFill { r: FRect, color: C4, width: f32 },
    Counter { c: Pt, n: u32, color: C4, d: f32, tail: Option<Pt> },
    Callout { r: FRect, tail: Pt, text: String, color: C4, size: f32, style: CalloutStyle, fill: bool },
    Spotlight { r: FRect },
    Magnify { src: Pt, dst: Pt, radius: f32, zoom: f32, color: C4, width: f32 },
}
