//! Tests only: the annotations.json reader of v0.3.0 (`json.rs` is
//! `git show v0.3.0:src/objects/json.rs` up to its tests module), over
//! this build's object types: v0.3.0's objects are this build's (no kind
//! was added since). Checks what v0.3 makes of the documents this one
//! writes (the crop's stand-ins).

#![allow(dead_code)]

pub use super::{CalloutStyle, FRect, Obj, Pt, TextFmt};

pub mod json;

pub mod edit {
    pub use crate::objects::edit::sanitize;
}