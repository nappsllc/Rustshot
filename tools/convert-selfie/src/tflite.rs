//! TensorFlow Lite (the `TFL3` flatbuffer, schema v3) to Rustshot's
//! segmentation model ([`crate::format::Model`]). Only what MediaPipe's
//! Selfie Segmentation needs is understood; anything else is refused by
//! name, so a different model fails loudly instead of converting wrong.
//!
//! - Weights: float32, or float16 (as such or behind `DEQUANTIZE`, which
//!   is folded away).
//! - Operators: CONV_2D, DEPTHWISE_CONV_2D, TRANSPOSE_CONV and MediaPipe's
//!   custom `Convolution2DTransposeBias`, ADD, MUL, AVERAGE_POOL_2D, MEAN
//!   (over height and width), RESIZE_BILINEAR, CONCATENATION (channels),
//!   PAD (height and width), RESHAPE, LOGISTIC, RELU, RELU6, HARD_SWISH.
//! - SAME padding becomes explicit (top, left) the way TensorFlow Lite
//!   works it out.

use crate::flat::{bad, Buf, Table, R};
use crate::format::{f16_to_f32, Act, Kind, Model, Op, Tensor};

// BuiltinOperator (schema.fbs).
const ADD: i32 = 0;
const AVERAGE_POOL_2D: i32 = 1;
const CONCATENATION: i32 = 2;
const CONV_2D: i32 = 3;
const DEPTHWISE_CONV_2D: i32 = 4;
const DEQUANTIZE: i32 = 6;
const LOGISTIC: i32 = 14;
const MUL: i32 = 18;
const RELU: i32 = 19;
const RELU6: i32 = 21;
const RESHAPE: i32 = 22;
const RESIZE_BILINEAR: i32 = 23;
const CUSTOM: i32 = 32;
const PAD: i32 = 34;
const MEAN: i32 = 40;
const TRANSPOSE_CONV: i32 = 67;
const HARD_SWISH: i32 = 117;

/// MediaPipe's transposed convolution with bias (a custom op).
pub const TRANSPOSE_CONV_BIAS: &str = "Convolution2DTransposeBias";

// TensorType.
const FLOAT32: i8 = 0;
const FLOAT16: i8 = 1;
const INT32: i8 = 2;

fn op_name(code: i32) -> String {
    let n = match code {
        ADD => "ADD",
        AVERAGE_POOL_2D => "AVERAGE_POOL_2D",
        CONCATENATION => "CONCATENATION",
        CONV_2D => "CONV_2D",
        DEPTHWISE_CONV_2D => "DEPTHWISE_CONV_2D",
        DEQUANTIZE => "DEQUANTIZE",
        LOGISTIC => "LOGISTIC",
        MUL => "MUL",
        RELU => "RELU",
        RELU6 => "RELU6",
        RESHAPE => "RESHAPE",
        RESIZE_BILINEAR => "RESIZE_BILINEAR",
        CUSTOM => "CUSTOM",
        PAD => "PAD",
        MEAN => "MEAN",
        TRANSPOSE_CONV => "TRANSPOSE_CONV",
        HARD_SWISH => "HARD_SWISH",
        5 => "DEPTH_TO_SPACE",
        9 => "FULLY_CONNECTED",
        17 => "MAX_POOL_2D",
        25 => "SOFTMAX",
        28 => "TANH",
        41 => "SUB",
        49 => "SPLIT",
        53 => "CAST",
        114 => "QUANTIZE",
        _ => return format!("builtin operator #{code}"),
    };
    n.to_string()
}

struct TfTensor {
    shape: Vec<i32>,
    ty: i8,
    buffer: u32,
    name: String,
}

/// What the conversion made, for the tool's summary.
#[derive(Debug, Default, PartialEq)]
pub struct Summary {
    /// Operators by name (after folding), in first-seen order.
    pub ops: Vec<(String, usize)>,
    pub weights: usize,
    pub input: Vec<u32>,
    pub output: Vec<u32>,
    pub folded_dequantize: usize,
    /// One line per operator (`--list`).
    pub listing: Vec<String>,
}

impl Summary {
    fn count(&mut self, name: &str) {
        match self.ops.iter_mut().find(|e| e.0 == name) {
            Some(e) => e.1 += 1,
            None => self.ops.push((name.to_string(), 1)),
        }
    }
}

struct Conv<'a> {
    file: Buf<'a>,
    tensors: Vec<TfTensor>,
    buffers: Vec<Table<'a>>,
    /// Our tensor for each TFLite tensor, once used.
    ours: Vec<Option<u16>>,
    /// DEQUANTIZE outputs (constants).
    folded: Vec<Option<Vec<f32>>>,
    m: Model,
}

impl<'a> Conv<'a> {
    fn tensor(&self, i: i32) -> R<&TfTensor> {
        usize::try_from(i).ok().and_then(|i| self.tensors.get(i)).map_or_else(|| bad(format!("tensor index {i} out of range")), Ok)
    }

    fn raw(&self, i: i32) -> R<&'a [u8]> {
        let t = self.tensor(i)?;
        let Some(b) = self.buffers.get(t.buffer as usize) else { return bad(format!("tensor {}: buffer {} out of range", t.name, t.buffer)) };
        let data = b.bytes(0)?;
        if !data.is_empty() {
            return Ok(data);
        }
        // Large models keep the data after the flatbuffer.
        let (off, size) = (b.u64_or(1, 0)?, b.u64_or(2, 0)?);
        if off > 1 && size > 0 {
            return self.file.bytes(off as usize, size as usize);
        }
        Ok(&[])
    }

    /// A constant's values (float32 / float16 / folded), or `None` for an
    /// activation.
    fn consts(&self, i: i32) -> R<Option<Vec<f32>>> {
        if let Some(Some(v)) = self.folded.get(i as usize) {
            return Ok(Some(v.clone()));
        }
        let raw = self.raw(i)?;
        if raw.is_empty() {
            return Ok(None);
        }
        let t = self.tensor(i)?;
        let n = t.shape.iter().try_fold(1usize, |a, &d| a.checked_mul(d.max(0) as usize));
        let Some(n) = n else { return bad(format!("tensor {}: shape {:?} too large", t.name, t.shape)) };
        let v: Vec<f32> = match t.ty {
            FLOAT32 => raw.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)).collect(),
            FLOAT16 => raw.as_chunks::<2>().0.iter().map(|b| f16_to_f32(u16::from_le_bytes(*b))).collect(),
            ty => return bad(format!("tensor {}: constant of type {ty} (only float32 and float16 weights are supported)", t.name)),
        };
        if v.len() != n {
            return bad(format!("tensor {}: {} values for shape {:?}", t.name, v.len(), t.shape));
        }
        Ok(Some(v))
    }

    fn ints(&self, i: i32) -> R<Vec<i32>> {
        let t = self.tensor(i)?;
        if t.ty != INT32 {
            return bad(format!("tensor {}: expected int32", t.name));
        }
        Ok(self.raw(i)?.as_chunks::<4>().0.iter().map(|b| i32::from_le_bytes(*b)).collect())
    }

    fn dims(&self, i: i32) -> R<Vec<u32>> {
        let t = self.tensor(i)?;
        t.shape.iter().map(|&d| u32::try_from(d).map_or_else(|_| bad(format!("tensor {}: dimension {d}", t.name)), Ok)).collect()
    }

    /// The (height, width, channels) of a 4-D NHWC tensor (batch 1).
    fn hwc(&self, i: i32) -> R<[u32; 3]> {
        match self.dims(i)?[..] {
            [1, h, w, c] => Ok([h, w, c]),
            ref d => bad(format!("tensor {}: shape {d:?}, expected 1xHxWxC", self.tensor(i)?.name)),
        }
    }

    /// Ours for TFLite tensor `i` (made on first use, constants with their
    /// values).
    fn ours(&mut self, i: i32) -> R<u16> {
        let ix = usize::try_from(i).map_or_else(|_| bad("optional tensor where one is needed"), Ok)?;
        if let Some(Some(o)) = self.ours.get(ix) {
            return Ok(*o);
        }
        let dims = self.dims(i)?;
        let data = self.consts(i)?;
        if self.m.tensors.len() >= u16::MAX as usize {
            return bad("too many tensors");
        }
        self.m.tensors.push(Tensor { dims, data });
        let o = (self.m.tensors.len() - 1) as u16;
        self.ours[ix] = Some(o);
        Ok(o)
    }

    fn push(&mut self, kind: Kind, act: Act, inputs: &[i32], output: i32) -> R<()> {
        let inputs = inputs.iter().map(|&i| self.ours(i)).collect::<R<Vec<u16>>>()?;
        let output = self.ours(output)?;
        self.m.ops.push(Op { kind, act, inputs, output });
        Ok(())
    }
}

fn act_of(code: i8) -> R<Act> {
    u8::try_from(code).ok().and_then(Act::from_code).map_or_else(|| bad(format!("fused activation {code} is not supported")), Ok)
}

/// TensorFlow Lite's padding before the first row (column) of a
/// convolution or pool: SAME (0) centres the window, VALID (1) has none.
fn conv_pad(padding: u8, n: u32, k: u32, s: u32, d: u32) -> R<u32> {
    // (In 64 bits: a damaged file's sizes must not overflow.)
    let (n, k, s, d) = (n as u64, k as u64, s.max(1) as u64, d as u64);
    match padding {
        0 => {
            let out = n.div_ceil(s);
            let eff = k.saturating_sub(1) * d + 1;
            let p = ((out.saturating_sub(1)) * s + eff).saturating_sub(n) / 2;
            u32::try_from(p).map_or_else(|_| bad("padding too large"), Ok)
        }
        1 => Ok(0),
        p => bad(format!("padding {p}")),
    }
}

/// A transposed convolution's padding off the output's top-left: SAME
/// trims `(in - 1)·s + k - out` evenly (the extra at the end), VALID none.
fn tconv_pad(same: bool, n_in: u32, k: u32, s: u32, n_out: u32) -> u32 {
    let full = (n_in.saturating_sub(1) as u64) * s as u64 + k as u64;
    if same { (full.saturating_sub(n_out as u64) / 2).min(u32::MAX as u64) as u32 } else { 0 }
}

fn positive(v: i32, what: &str) -> R<u32> {
    if v > 0 { Ok(v as u32) } else { bad(format!("{what} {v}")) }
}

/// Convert a `.tflite` file.
pub fn convert(file: &[u8]) -> R<(Model, Summary)> {
    let buf = Buf(file);
    if file.len() < 8 || &file[4..8] != b"TFL3" {
        return bad("not a TensorFlow Lite model (no TFL3 identifier)");
    }
    let root = buf.root()?;
    let codes: Vec<(i32, String)> = root
        .tables(1)?
        .iter()
        .map(|c| Ok(((c.i8_or(0, 0)? as i32).max(c.i32_or(3, 0)?), c.string(1)?)))
        .collect::<R<_>>()?;
    let subgraphs = root.tables(2)?;
    let Some(sg) = subgraphs.first() else { return bad("no subgraph") };
    if subgraphs.len() > 1 {
        return bad(format!("{} subgraphs (one expected)", subgraphs.len()));
    }
    let tensors: Vec<TfTensor> = sg
        .tables(0)?
        .iter()
        .map(|t| Ok(TfTensor { shape: t.i32s(0)?, ty: t.i8_or(1, 0)?, buffer: t.u32_or(2, 0)?, name: t.string(3)? }))
        .collect::<R<_>>()?;
    let n = tensors.len();
    let mut c = Conv {
        file: buf,
        tensors,
        buffers: root.tables(4)?,
        ours: vec![None; n],
        folded: vec![None; n],
        m: Model { tensors: Vec::new(), ops: Vec::new(), input: 0, output: 0 },
    };
    let (inputs, outputs) = (sg.i32s(1)?, sg.i32s(2)?);
    let (&[gin], &[gout]) = (&inputs[..], &outputs[..]) else { return bad(format!("{} inputs and {} outputs (one each expected)", inputs.len(), outputs.len())) };
    let mut sum = Summary::default();
    c.m.input = c.ours(gin)?;
    for (k, op) in sg.tables(3)?.iter().enumerate() {
        let code = c_get(&codes, op.u32_or(0, 0)? as usize)?;
        let ins = op.i32s(1)?;
        let outs = op.i32s(2)?;
        let &[out] = &outs[..] else { return bad(format!("operator {k}: {} outputs", outs.len())) };
        let opts = op.table(4)?;
        let name = if code.0 == CUSTOM { code.1.clone() } else { op_name(code.0) };
        let e = |s: &str| crate::flat::BadFile(format!("operator {k} ({name}): {s}"));
        let need = |n: usize| if ins.len() < n { Err(e(&format!("{} inputs", ins.len()))) } else { Ok(()) };
        // The listing: each operator, its tensors (name and shape), the
        // custom options' bytes.
        {
            let desc = |i: &i32| c.tensor(*i).map_or_else(|_| format!("#{i}"), |t| format!("{}{:?}", t.name, t.shape));
            let custom = op.bytes(5)?;
            let tail = if custom.is_empty() { String::new() } else { format!("  custom options {custom:02x?}") };
            sum.listing.push(format!("{k:3} {name} [{}] -> {}{tail}", ins.iter().map(desc).collect::<Vec<_>>().join(", "), desc(&out)));
        }
        // The options union's tag must be the table the operator reads.
        let want_tag = match code.0 {
            CONV_2D => Some(1),
            DEPTHWISE_CONV_2D => Some(2),
            AVERAGE_POOL_2D => Some(5),
            CONCATENATION => Some(10),
            ADD => Some(11),
            RESIZE_BILINEAR => Some(15),
            MUL => Some(21),
            MEAN => Some(27),
            TRANSPOSE_CONV => Some(49),
            _ => None,
        };
        if let Some(t) = want_tag
            && opts.is_some()
            && op.u8_or(3, 0)? != t
        {
            return Err(e(&format!("options of type {} (expected {t})", op.u8_or(3, 0)?)));
        }
        // Options: absent fields take the schema's defaults.
        let o_u8 = |id, d| opts.map_or(Ok(d), |t| t.u8_or(id, d));
        let o_i8 = |id, d| opts.map_or(Ok(d), |t| t.i8_or(id, d));
        let o_i32 = |id, d| opts.map_or(Ok(d), |t| t.i32_or(id, d));
        let o_bool = |id, d| opts.map_or(Ok(d), |t| t.bool_or(id, d));
        match code.0 {
            DEQUANTIZE => {
                need(1)?;
                let Some(v) = c.consts(ins[0])? else { return Err(e("dequantizes an activation (only constant weights can be)")) };
                c.tensor(out)?;
                c.folded[out as usize] = Some(v);
                sum.folded_dequantize += 1;
                continue;
            }
            CONV_2D | DEPTHWISE_CONV_2D => {
                need(2)?;
                let dw = code.0 == DEPTHWISE_CONV_2D;
                let (sw, sh) = (positive(o_i32(1, 0)?, "stride")?, positive(o_i32(2, 0)?, "stride")?);
                let act = act_of(o_i8(if dw { 4 } else { 3 }, 0)?)?;
                let (dlw, dlh) = (positive(o_i32(if dw { 5 } else { 4 }, 1)?, "dilation")?, positive(o_i32(if dw { 6 } else { 5 }, 1)?, "dilation")?);
                let x = c.hwc(ins[0])?;
                let w = c.dims(ins[1])?;
                let [_, kh, kw, _] = w[..] else { return Err(e("weights are not 4-D")) };
                let padding = o_u8(0, 0)?;
                let pad = [conv_pad(padding, x[0], kh, sh, dlh)?, conv_pad(padding, x[1], kw, sw, dlw)?];
                let kind = if dw {
                    let oc = c.hwc(out)?[2];
                    if x[2] == 0 || oc % x[2] != 0 {
                        return Err(e("output channels are not a multiple of the input's"));
                    }
                    Kind::Depthwise { stride: [sh, sw], pad, dil: [dlh, dlw], mult: oc / x[2] }
                } else {
                    Kind::Conv { stride: [sh, sw], pad, dil: [dlh, dlw] }
                };
                let ins: Vec<i32> = ins.iter().copied().take(3).filter(|&i| i >= 0).collect();
                c.push(kind, act, &ins, out)?;
            }
            TRANSPOSE_CONV => {
                // (output_shape, weights, input[, bias])
                need(3)?;
                let same = o_u8(0, 0)? == 0;
                let (sw, sh) = (positive(o_i32(1, 0)?, "stride")?, positive(o_i32(2, 0)?, "stride")?);
                let act = act_of(o_i8(3, 0)?)?;
                let (x, y, w) = (c.hwc(ins[2])?, c.hwc(out)?, c.dims(ins[1])?);
                let [_, kh, kw, _] = w[..] else { return Err(e("weights are not 4-D")) };
                let pad = [tconv_pad(same, x[0], kh, sh, y[0]), tconv_pad(same, x[1], kw, sw, y[1])];
                let mut tins = vec![ins[2], ins[1]];
                if ins.len() > 3 && ins[3] >= 0 {
                    tins.push(ins[3]);
                }
                c.push(Kind::TransposeConv { stride: [sh, sw], pad }, act, &tins, out)?;
            }
            CUSTOM if code.1 == TRANSPOSE_CONV_BIAS => {
                // (input, weights, bias); options: TfLiteTransposeConvParams
                // as raw bytes (padding: 1 SAME, 2 VALID; stride w, h).
                need(3)?;
                let raw = op.bytes(5)?;
                if raw.len() < 12 {
                    return Err(e("custom options too short"));
                }
                let rd = |i: usize| i32::from_le_bytes(raw[i * 4..i * 4 + 4].try_into().unwrap());
                let same = match rd(0) {
                    1 => true,
                    2 => false,
                    p => return Err(e(&format!("padding {p}"))),
                };
                let (sw, sh) = (positive(rd(1), "stride")?, positive(rd(2), "stride")?);
                let (x, y, w) = (c.hwc(ins[0])?, c.hwc(out)?, c.dims(ins[1])?);
                let [_, kh, kw, _] = w[..] else { return Err(e("weights are not 4-D")) };
                let pad = [tconv_pad(same, x[0], kh, sh, y[0]), tconv_pad(same, x[1], kw, sw, y[1])];
                c.push(Kind::TransposeConv { stride: [sh, sw], pad }, Act::None, &ins[..3], out)?;
            }
            ADD | MUL => {
                need(2)?;
                let act = act_of(o_i8(0, 0)?)?;
                c.push(if code.0 == ADD { Kind::Add } else { Kind::Mul }, act, &ins[..2], out)?;
            }
            AVERAGE_POOL_2D => {
                need(1)?;
                let (sw, sh) = (positive(o_i32(1, 0)?, "stride")?, positive(o_i32(2, 0)?, "stride")?);
                let (fw, fh) = (positive(o_i32(3, 0)?, "filter")?, positive(o_i32(4, 0)?, "filter")?);
                let act = act_of(o_i8(5, 0)?)?;
                let x = c.hwc(ins[0])?;
                let padding = o_u8(0, 0)?;
                let pad = [conv_pad(padding, x[0], fh, sh, 1)?, conv_pad(padding, x[1], fw, sw, 1)?];
                c.push(Kind::AvgPool { stride: [sh, sw], pad, size: [fh, fw] }, act, &ins[..1], out)?;
            }
            MEAN => {
                need(2)?;
                let mut axes = c.ints(ins[1])?;
                axes.sort_unstable();
                if axes != [1, 2] {
                    return Err(e(&format!("axes {axes:?} (only height and width)")));
                }
                c.push(Kind::Mean, Act::None, &ins[..1], out)?;
            }
            RESIZE_BILINEAR => {
                need(1)?;
                let (align, half) = (o_bool(2, false)?, o_bool(3, false)?);
                c.push(Kind::Resize { align, half }, Act::None, &ins[..1], out)?;
            }
            CONCATENATION => {
                let axis = o_i32(0, 0)?;
                let act = act_of(o_i8(1, 0)?)?;
                let rank = c.dims(out)?.len() as i32;
                if axis != 3 && axis != -1 || rank != 4 {
                    return Err(e(&format!("axis {axis} (only channels)")));
                }
                c.push(Kind::Concat, act, &ins, out)?;
            }
            PAD => {
                need(2)?;
                let p = c.ints(ins[1])?;
                let &[n0, n1, t, b, l, r, c0, c1] = &p[..] else { return Err(e("paddings are not 4x2")) };
                if [n0, n1, c0, c1] != [0; 4] || [t, b, l, r].iter().any(|&v| v < 0) {
                    return Err(e(&format!("paddings {p:?} (only height and width)")));
                }
                c.push(Kind::Pad { pad: [t as u32, b as u32, l as u32, r as u32] }, Act::None, &ins[..1], out)?;
            }
            RESHAPE => {
                need(1)?;
                c.push(Kind::Reshape, Act::None, &ins[..1], out)?;
            }
            LOGISTIC | RELU | RELU6 | HARD_SWISH => {
                need(1)?;
                let kind = match code.0 {
                    LOGISTIC => Kind::Sigmoid,
                    RELU => Kind::Relu,
                    RELU6 => Kind::Relu6,
                    _ => Kind::HardSwish,
                };
                c.push(kind, Act::None, &ins[..1], out)?;
            }
            _ => return bad(format!("operator {k}: {name} is not supported")),
        }
        sum.count(&name);
    }
    c.m.output = c.ours(gout)?;
    sum.weights = c.m.tensors.iter().filter_map(|t| t.data.as_ref()).map(Vec::len).sum();
    sum.input = c.m.tensors[c.m.input as usize].dims.clone();
    sum.output = c.m.tensors[c.m.output as usize].dims.clone();
    Ok((c.m, sum))
}

fn c_get(codes: &[(i32, String)], i: usize) -> R<&(i32, String)> {
    codes.get(i).map_or_else(|| bad(format!("operator code {i} out of range")), Ok)
}
