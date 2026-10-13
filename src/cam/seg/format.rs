//! `selfie.bin`: the person-segmentation model as Rustshot keeps it. A
//! compact list of tensors (shapes, constant data as little-endian f16)
//! and the operators between them, nothing else; written by
//! `tools/convert-selfie` from MediaPipe's `.tflite` and read by
//! [`super::Net`]. This file is shared with that tool (`#[path]`), so it
//! uses only `std`.
//!
//! Layout (little-endian):
//!
//! ```text
//! "RSEG"  u16 version (1)  u16 tensors  u16 ops  u16 input  u16 output
//! tensor: u8 rank, u32 × rank dims, u8 has_data, then f16 × product(dims)
//! op:     u8 code, u8 act, u8 n_in, u16 × n_in inputs, u16 output,
//!         u8 n_params, i32 × n_params
//! ```
//!
//! Activations are NHWC with a batch of 1. Constant layouts follow
//! TensorFlow Lite: convolution weights OHWI, depthwise weights 1HWC,
//! transposed-convolution weights OHWI, biases `[C]`.
#![allow(dead_code)] // shared with the converter, which uses other parts

use std::fmt;

pub const MAGIC: &[u8; 4] = b"RSEG";
pub const VERSION: u16 = 1;

/// A fused (or standalone) activation; TensorFlow Lite's numbering.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    None = 0,
    Relu = 1,
    ReluN1To1 = 2,
    Relu6 = 3,
    Tanh = 4,
}

impl Act {
    pub fn from_code(c: u8) -> Option<Act> {
        Some(match c {
            0 => Act::None,
            1 => Act::Relu,
            2 => Act::ReluN1To1,
            3 => Act::Relu6,
            4 => Act::Tanh,
            _ => return None,
        })
    }
}

/// What an operator does. Padding is explicit (top, left), worked out by
/// the converter the way TensorFlow Lite does for SAME / VALID.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Inputs: x, weights OHWI, [bias].
    Conv { stride: [u32; 2], pad: [u32; 2], dil: [u32; 2] },
    /// Inputs: x, weights 1HW(C·mult), [bias].
    Depthwise { stride: [u32; 2], pad: [u32; 2], dil: [u32; 2], mult: u32 },
    /// Elementwise with broadcasting (NHWC, from the right).
    Add,
    Mul,
    /// Average over the valid part of each window.
    AvgPool { stride: [u32; 2], pad: [u32; 2], size: [u32; 2] },
    /// The mean over height and width (dims kept).
    Mean,
    /// To the output tensor's size; TensorFlow's flags.
    Resize { align: bool, half: bool },
    /// Inputs: x, weights OHWI, [bias]; to the output tensor's size.
    TransposeConv { stride: [u32; 2], pad: [u32; 2] },
    Sigmoid,
    Relu,
    Relu6,
    HardSwish,
    /// Along the channels.
    Concat,
    /// The same values under another shape.
    Reshape,
    /// Zeros around the picture: rows above and below, columns left and
    /// right (channels are not padded).
    Pad { pad: [u32; 4] },
}

impl Kind {
    fn code(&self) -> u8 {
        match self {
            Kind::Conv { .. } => 1,
            Kind::Depthwise { .. } => 2,
            Kind::Add => 3,
            Kind::Mul => 4,
            Kind::AvgPool { .. } => 5,
            Kind::Mean => 6,
            Kind::Resize { .. } => 7,
            Kind::TransposeConv { .. } => 8,
            Kind::Sigmoid => 9,
            Kind::Relu => 10,
            Kind::Relu6 => 11,
            Kind::HardSwish => 12,
            Kind::Concat => 13,
            Kind::Reshape => 14,
            Kind::Pad { .. } => 15,
        }
    }

    fn params(&self) -> Vec<i32> {
        let v = |a: &[u32]| a.iter().map(|&x| x as i32).collect::<Vec<_>>();
        match *self {
            Kind::Conv { stride, pad, dil } => v(&[stride[0], stride[1], pad[0], pad[1], dil[0], dil[1]]),
            Kind::Depthwise { stride, pad, dil, mult } => v(&[stride[0], stride[1], pad[0], pad[1], dil[0], dil[1], mult]),
            Kind::AvgPool { stride, pad, size } => v(&[stride[0], stride[1], pad[0], pad[1], size[0], size[1]]),
            Kind::Resize { align, half } => vec![align as i32, half as i32],
            Kind::TransposeConv { stride, pad } => v(&[stride[0], stride[1], pad[0], pad[1]]),
            Kind::Pad { pad } => v(&pad),
            _ => Vec::new(),
        }
    }

    fn from_parts(code: u8, p: &[i32]) -> Option<Kind> {
        let u = |i: usize| p.get(i).and_then(|&x| u32::try_from(x).ok());
        let pair = |i: usize| Some([u(i)?, u(i + 1)?]);
        Some(match code {
            1 => Kind::Conv { stride: pair(0)?, pad: pair(2)?, dil: pair(4)? },
            2 => Kind::Depthwise { stride: pair(0)?, pad: pair(2)?, dil: pair(4)?, mult: u(6)? },
            3 => Kind::Add,
            4 => Kind::Mul,
            5 => Kind::AvgPool { stride: pair(0)?, pad: pair(2)?, size: pair(4)? },
            6 => Kind::Mean,
            7 => Kind::Resize { align: u(0)? != 0, half: u(1)? != 0 },
            8 => Kind::TransposeConv { stride: pair(0)?, pad: pair(2)? },
            9 => Kind::Sigmoid,
            10 => Kind::Relu,
            11 => Kind::Relu6,
            12 => Kind::HardSwish,
            13 => Kind::Concat,
            14 => Kind::Reshape,
            15 => Kind::Pad { pad: [u(0)?, u(1)?, u(2)?, u(3)?] },
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Op {
    pub kind: Kind,
    pub act: Act,
    pub inputs: Vec<u16>,
    pub output: u16,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Tensor {
    pub dims: Vec<u32>,
    /// Constant values (weights, biases); `None` for activations.
    pub data: Option<Vec<f32>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Model {
    pub tensors: Vec<Tensor>,
    pub ops: Vec<Op>,
    pub input: u16,
    pub output: u16,
}

#[derive(Debug, PartialEq, Eq)]
pub struct FormatError(pub String);

impl fmt::Display for FormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "selfie model: {}", self.0)
    }
}

impl std::error::Error for FormatError {}

fn bad(s: impl Into<String>) -> FormatError {
    FormatError(s.into())
}

/// IEEE half to single (exact).
pub fn f16_to_f32(h: u16) -> f32 {
    let sign = if h & 0x8000 != 0 { -1.0f32 } else { 1.0 };
    let e = (h >> 10) & 0x1f;
    let m = (h & 0x3ff) as u32;
    match e {
        0 => sign * m as f32 * (1.0 / 16_777_216.0), // 2^-24
        31 => f32::from_bits(((h as u32 & 0x8000) << 16) | 0x7f80_0000 | (m << 13)),
        _ => f32::from_bits(((h as u32 & 0x8000) << 16) | ((e as u32 + 112) << 23) | (m << 13)),
    }
}

/// Single to IEEE half, rounded to nearest even (overflow to infinity).
pub fn f32_to_f16(x: f32) -> u16 {
    let b = x.to_bits();
    let s = ((b >> 16) & 0x8000) as u16;
    let e = ((b >> 23) & 0xff) as i32;
    let m = b & 0x7f_ffff;
    if e == 255 {
        return s | 0x7c00 | if m != 0 { 0x200 } else { 0 };
    }
    let e16 = e - 112;
    if e16 >= 31 {
        return s | 0x7c00;
    }
    let round = |v: u32, rem: u32, half: u32| if rem > half || (rem == half && v & 1 == 1) { v + 1 } else { v };
    if e16 <= 0 {
        if e16 < -10 {
            return s;
        }
        let m = m | 0x80_0000;
        let shift = (14 - e16) as u32;
        let v = round(m >> shift, m & ((1 << shift) - 1), 1 << (shift - 1));
        return s | v as u16;
    }
    // A carry out of the mantissa steps the exponent (to infinity at the top).
    s | round(((e16 as u32) << 10) | (m >> 13), m & 0x1fff, 0x1000) as u16
}

impl Model {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut o = Vec::new();
        o.extend_from_slice(MAGIC);
        for v in [VERSION, self.tensors.len() as u16, self.ops.len() as u16, self.input, self.output] {
            o.extend_from_slice(&v.to_le_bytes());
        }
        for t in &self.tensors {
            o.push(t.dims.len() as u8);
            for d in &t.dims {
                o.extend_from_slice(&d.to_le_bytes());
            }
            match &t.data {
                Some(d) => {
                    o.push(1);
                    for &x in d {
                        o.extend_from_slice(&f32_to_f16(x).to_le_bytes());
                    }
                }
                None => o.push(0),
            }
        }
        for op in &self.ops {
            o.push(op.kind.code());
            o.push(op.act as u8);
            o.push(op.inputs.len() as u8);
            for i in &op.inputs {
                o.extend_from_slice(&i.to_le_bytes());
            }
            o.extend_from_slice(&op.output.to_le_bytes());
            let p = op.kind.params();
            o.push(p.len() as u8);
            for x in p {
                o.extend_from_slice(&x.to_le_bytes());
            }
        }
        o
    }

    /// Parse `selfie.bin`; every index and size is checked.
    pub fn parse(b: &[u8]) -> Result<Model, FormatError> {
        let mut r = Reader { b, at: 0 };
        if r.take(4)? != MAGIC {
            return Err(bad("not a Rustshot segmentation model"));
        }
        let version = r.u16()?;
        if version != VERSION {
            return Err(bad(format!("version {version} (this build reads {VERSION})")));
        }
        let (nt, no, input, output) = (r.u16()?, r.u16()?, r.u16()?, r.u16()?);
        let mut tensors = Vec::with_capacity(nt as usize);
        for i in 0..nt {
            let rank = r.u8()? as usize;
            if rank > 4 {
                return Err(bad(format!("tensor {i} has rank {rank}")));
            }
            let dims = (0..rank).map(|_| r.u32()).collect::<Result<Vec<_>, _>>()?;
            let n = dims.iter().try_fold(1usize, |a, &d| a.checked_mul(d as usize)).filter(|&n| n <= 1 << 26);
            let n = n.ok_or_else(|| bad(format!("tensor {i} is too large")))?;
            let data = match r.u8()? {
                0 => None,
                1 => Some(r.take(n.checked_mul(2).ok_or_else(|| bad("size"))?)?.as_chunks::<2>().0.iter().map(|h| f16_to_f32(u16::from_le_bytes(*h))).collect()),
                k => return Err(bad(format!("tensor {i}: data flag {k}"))),
            };
            tensors.push(Tensor { dims, data });
        }
        let mut ops = Vec::with_capacity(no as usize);
        for i in 0..no {
            let (code, act, n_in) = (r.u8()?, r.u8()?, r.u8()? as usize);
            let inputs = (0..n_in).map(|_| r.u16()).collect::<Result<Vec<_>, _>>()?;
            let out = r.u16()?;
            let np = r.u8()? as usize;
            let params = (0..np).map(|_| r.u32().map(|x| x as i32)).collect::<Result<Vec<_>, _>>()?;
            let kind = Kind::from_parts(code, &params).ok_or_else(|| bad(format!("op {i}: code {code} with {params:?}")))?;
            let act = Act::from_code(act).ok_or_else(|| bad(format!("op {i}: activation {act}")))?;
            if inputs.iter().chain([&out]).any(|&t| t >= nt) {
                return Err(bad(format!("op {i}: tensor index out of range")));
            }
            ops.push(Op { kind, act, inputs, output: out });
        }
        if input >= nt || output >= nt {
            return Err(bad("input or output out of range"));
        }
        if r.at != b.len() {
            return Err(bad(format!("{} trailing bytes", b.len() - r.at)));
        }
        Ok(Model { tensors, ops, input, output })
    }
}

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], FormatError> {
        let end = self.at.checked_add(n).filter(|&e| e <= self.b.len()).ok_or_else(|| bad("truncated"))?;
        let s = &self.b[self.at..end];
        self.at = end;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, FormatError> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, FormatError> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32, FormatError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
}
