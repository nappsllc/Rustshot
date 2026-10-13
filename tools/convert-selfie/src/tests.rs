//! Converting `.tflite` files made here: a small FlatBuffers writer
//! lays out TensorFlow Lite's schema (Model, OperatorCode, SubGraph,
//! Tensor, Buffer, Operator and the options tables, by field id).

use crate::format::{f32_to_f16, Act, Kind, Model};
use crate::tflite::{convert, TRANSPOSE_CONV_BIAS};

/// A field value.
enum F {
    U8(u8),
    I8(i8),
    I32(i32),
    U32(u32),
    Table(Tab),
    Bytes(Vec<u8>),
    I32s(Vec<i32>),
    Tables(Vec<Tab>),
    Str(String),
}

/// A table: (field id, value).
#[derive(Default)]
struct Tab(Vec<(usize, F)>);

impl Tab {
    fn f(mut self, id: usize, v: F) -> Tab {
        self.0.push((id, v));
        self
    }
}

fn align(buf: &mut Vec<u8>) {
    while !buf.len().is_multiple_of(4) {
        buf.push(0);
    }
}

fn put32(buf: &mut [u8], at: usize, v: u32) {
    buf[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

/// Each table: its vtable, then the table (an 8-byte slot per field);
/// what it points to comes after it (offsets point forward).
fn emit_table(buf: &mut Vec<u8>, t: &Tab) -> usize {
    align(buf);
    let n = t.0.iter().map(|f| f.0 + 1).max().unwrap_or(0);
    let vt = buf.len();
    buf.extend_from_slice(&((4 + 2 * n) as u16).to_le_bytes());
    buf.extend_from_slice(&((4 + 8 * t.0.len()) as u16).to_le_bytes());
    for id in 0..n {
        let off = t.0.iter().position(|f| f.0 == id).map_or(0, |k| 4 + 8 * k);
        buf.extend_from_slice(&(off as u16).to_le_bytes());
    }
    align(buf);
    let pos = buf.len();
    buf.extend_from_slice(&((pos - vt) as i32).to_le_bytes());
    buf.resize(pos + 4 + 8 * t.0.len(), 0);
    let mut later = Vec::new();
    for (k, (_, v)) in t.0.iter().enumerate() {
        let slot = pos + 4 + 8 * k;
        match v {
            F::U8(x) => buf[slot] = *x,
            F::I8(x) => buf[slot] = *x as u8,
            F::I32(x) => put32(buf, slot, *x as u32),
            F::U32(x) => put32(buf, slot, *x),
            _ => later.push((slot, v)),
        }
    }
    for (slot, v) in later {
        let at = emit(buf, v);
        put32(buf, slot, (at - slot) as u32);
    }
    pos
}

fn emit(buf: &mut Vec<u8>, v: &F) -> usize {
    align(buf);
    let pos = buf.len();
    match v {
        F::Table(t) => return emit_table(buf, t),
        F::Bytes(b) => {
            buf.extend_from_slice(&(b.len() as u32).to_le_bytes());
            buf.extend_from_slice(b);
        }
        F::Str(s) => {
            buf.extend_from_slice(&(s.len() as u32).to_le_bytes());
            buf.extend_from_slice(s.as_bytes());
            buf.push(0);
        }
        F::I32s(v) => {
            buf.extend_from_slice(&(v.len() as u32).to_le_bytes());
            for x in v {
                buf.extend_from_slice(&x.to_le_bytes());
            }
        }
        F::Tables(ts) => {
            buf.extend_from_slice(&(ts.len() as u32).to_le_bytes());
            let slots = buf.len();
            buf.resize(slots + 4 * ts.len(), 0);
            for (k, t) in ts.iter().enumerate() {
                let at = emit_table(buf, t);
                put32(buf, slots + 4 * k, (at - (slots + 4 * k)) as u32);
            }
        }
        F::U8(_) | F::I8(_) | F::I32(_) | F::U32(_) => unreachable!("scalars live in their table"),
    }
    pos
}

fn finish(root: &Tab) -> Vec<u8> {
    let mut buf = vec![0u8; 8];
    buf[4..8].copy_from_slice(b"TFL3");
    let at = emit_table(&mut buf, root);
    put32(&mut buf, 0, at as u32);
    buf
}

/// A model under construction: tensors, buffers (0 is the empty one),
/// operator codes and operators.
struct M {
    tensors: Vec<Tab>,
    buffers: Vec<Vec<u8>>,
    codes: Vec<(i32, String)>,
    ops: Vec<Tab>,
    seed: u32,
}

const F32: i8 = 0;
const F16: i8 = 1;
const I32: i8 = 2;
const U8T: i8 = 3;

impl M {
    fn new() -> M {
        M { tensors: Vec::new(), buffers: vec![Vec::new()], codes: Vec::new(), ops: Vec::new(), seed: 7 }
    }

    fn rnd(&mut self) -> f32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        (self.seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5
    }

    fn tensor(&mut self, shape: &[i32], ty: i8, data: Vec<u8>) -> i32 {
        let buffer = if data.is_empty() { 0 } else { self.buffers.push(data); self.buffers.len() as u32 - 1 };
        let name = format!("t{}", self.tensors.len());
        self.tensors.push(Tab::default().f(0, F::I32s(shape.to_vec())).f(1, F::I8(ty)).f(2, F::U32(buffer)).f(3, F::Str(name)));
        self.tensors.len() as i32 - 1
    }

    fn act(&mut self, shape: &[i32]) -> i32 {
        self.tensor(shape, F32, Vec::new())
    }

    /// Random float32 weights (exact in f16, so they compare).
    fn weights(&mut self, shape: &[i32]) -> (i32, Vec<f32>) {
        let n: i32 = shape.iter().product();
        let v: Vec<f32> = (0..n).map(|_| crate::format::f16_to_f32(f32_to_f16(self.rnd()))).collect();
        let raw = v.iter().flat_map(|x| x.to_le_bytes()).collect();
        (self.tensor(shape, F32, raw), v)
    }

    /// Float16 weights behind a DEQUANTIZE (as MediaPipe ships them).
    fn weights_f16(&mut self, shape: &[i32]) -> (i32, Vec<f32>) {
        let n: i32 = shape.iter().product();
        let h: Vec<u16> = (0..n).map(|_| f32_to_f16(self.rnd())).collect();
        let raw = h.iter().flat_map(|x| x.to_le_bytes()).collect();
        let q = self.tensor(shape, F16, raw);
        let out = self.act(shape);
        self.op(6, "", &[q], out, None, None);
        (out, h.iter().map(|&x| crate::format::f16_to_f32(x)).collect())
    }

    fn ints(&mut self, v: &[i32]) -> i32 {
        let raw = v.iter().flat_map(|x| x.to_le_bytes()).collect();
        self.tensor(&[v.len() as i32], I32, raw)
    }

    fn op(&mut self, code: i32, custom: &str, ins: &[i32], out: i32, opts: Option<Tab>, custom_opts: Option<Vec<u8>>) {
        let idx = match self.codes.iter().position(|c| c.0 == code && c.1 == custom) {
            Some(i) => i,
            None => {
                self.codes.push((code, custom.to_string()));
                self.codes.len() - 1
            }
        };
        let mut t = Tab::default().f(0, F::U32(idx as u32)).f(1, F::I32s(ins.to_vec())).f(2, F::I32s(vec![out]));
        if let Some(o) = opts {
            // The BuiltinOptions union's tag for the operator's table.
            let tag = match code {
                3 => 1,
                4 => 2,
                1 => 5,
                2 => 10,
                0 => 11,
                23 => 15,
                18 => 21,
                40 => 27,
                67 => 49,
                _ => 0,
            };
            t = t.f(3, F::U8(tag)).f(4, F::Table(o));
        }
        if let Some(c) = custom_opts {
            t = t.f(5, F::Bytes(c));
        }
        self.ops.push(t);
    }

    fn file(self, input: i32, output: i32) -> Vec<u8> {
        // Codes as newer converters write them: the deprecated byte (capped
        // at 127) and the full code.
        let codes = self
            .codes
            .iter()
            .map(|(c, s)| {
                let t = Tab::default().f(0, F::I8((*c).min(127) as i8)).f(2, F::I32(1)).f(3, F::I32(*c));
                if s.is_empty() { t } else { t.f(1, F::Str(s.clone())) }
            })
            .collect();
        let sg = Tab::default().f(0, F::Tables(self.tensors)).f(1, F::I32s(vec![input])).f(2, F::I32s(vec![output])).f(3, F::Tables(self.ops));
        let buffers = self.buffers.into_iter().map(|b| if b.is_empty() { Tab::default() } else { Tab::default().f(0, F::Bytes(b)) }).collect();
        finish(&Tab::default().f(0, F::U32(3)).f(1, F::Tables(codes)).f(2, F::Tables(vec![sg])).f(4, F::Tables(buffers)))
    }
}

/// Conv2DOptions: padding (0 SAME, 1 VALID), stride w, h, activation.
fn conv_opts(valid: bool, s: i32, act: i8) -> Tab {
    Tab::default().f(0, F::U8(valid as u8)).f(1, F::I32(s)).f(2, F::I32(s)).f(3, F::I8(act))
}

fn data(m: &Model, t: u16) -> &[f32] {
    m.tensors[t as usize].data.as_deref().expect("a constant")
}

#[test]
fn a_mobile_head_converts() {
    let mut m = M::new();
    let x = m.act(&[1, 9, 9, 3]);
    // Conv 3x3 / 2, SAME, f16 weights behind DEQUANTIZE, ReLU6.
    let (w, wv) = m.weights_f16(&[4, 3, 3, 3]);
    let (b, bv) = m.weights(&[4]);
    let c1 = m.act(&[1, 5, 5, 4]);
    m.op(3, "", &[x, w, b], c1, Some(conv_opts(false, 2, 3)), None);
    let h = m.act(&[1, 5, 5, 4]);
    m.op(117, "", &[c1], h, None, None);
    // Depthwise 3x3, multiplier from the channels (the option says 0).
    let (dw, _) = m.weights(&[1, 3, 3, 8]);
    let d1 = m.act(&[1, 5, 5, 8]);
    let dopts = Tab::default().f(0, F::U8(0)).f(1, F::I32(1)).f(2, F::I32(1)).f(3, F::I32(0)).f(4, F::I8(1));
    m.op(4, "", &[h, dw, -1], d1, Some(dopts), None);
    // Squeeze and excite: MEAN over 1, 2; then a sigmoid and the product.
    let axes = m.ints(&[2, 1]);
    let g = m.act(&[1, 1, 1, 8]);
    m.op(40, "", &[d1, axes], g, Some(Tab::default().f(0, F::U8(1))), None);
    let s = m.act(&[1, 1, 1, 8]);
    m.op(14, "", &[g], s, None, None);
    let p = m.act(&[1, 5, 5, 8]);
    m.op(18, "", &[d1, s], p, Some(Tab::default()), None);
    // PAD then a VALID 2x2 average pool.
    let pads = m.ints(&[0, 0, 0, 1, 0, 1, 0, 0]);
    let pd = m.act(&[1, 6, 6, 8]);
    m.op(34, "", &[p, pads], pd, None, None);
    let ap = m.act(&[1, 3, 3, 8]);
    m.op(1, "", &[pd], ap, Some(Tab::default().f(0, F::U8(1)).f(1, F::I32(2)).f(2, F::I32(2)).f(3, F::I32(2)).f(4, F::I32(2))), None);
    // Up again (half-pixel bilinear), concatenated with the skip.
    let size = m.ints(&[5, 5]);
    let up = m.act(&[1, 5, 5, 8]);
    m.op(23, "", &[ap, size], up, Some(Tab::default().f(3, F::U8(1))), None);
    let cat = m.act(&[1, 5, 5, 16]);
    m.op(2, "", &[up, p], cat, Some(Tab::default().f(0, F::I32(-1))), None);
    // MediaPipe's transposed convolution with bias, SAME, 2x.
    let (tw, _) = m.weights(&[1, 2, 2, 16]);
    let (tb, _) = m.weights(&[1]);
    let t = m.act(&[1, 10, 10, 1]);
    let raw: Vec<u8> = [1i32, 2, 2].iter().flat_map(|v| v.to_le_bytes()).collect();
    m.op(32, TRANSPOSE_CONV_BIAS, &[cat, tw, tb], t, None, Some(raw));
    let out = m.act(&[1, 10, 10, 1]);
    m.op(14, "", &[t], out, None, None);
    let file = m.file(x, out);

    let (model, sum) = convert(&file).expect("converts");
    assert_eq!(sum.folded_dequantize, 1);
    assert_eq!((sum.input.clone(), sum.output.clone()), (vec![1, 9, 9, 3], vec![1, 10, 10, 1]));
    let kinds: Vec<Kind> = model.ops.iter().map(|o| o.kind).collect();
    assert_eq!(
        kinds,
        [
            Kind::Conv { stride: [2, 2], pad: [1, 1], dil: [1, 1] },
            Kind::HardSwish,
            Kind::Depthwise { stride: [1, 1], pad: [1, 1], dil: [1, 1], mult: 2 },
            Kind::Mean,
            Kind::Sigmoid,
            Kind::Mul,
            Kind::Pad { pad: [0, 1, 0, 1] },
            Kind::AvgPool { stride: [2, 2], pad: [0, 0], size: [2, 2] },
            Kind::Resize { align: false, half: true },
            Kind::Concat,
            Kind::TransposeConv { stride: [2, 2], pad: [0, 0] },
            Kind::Sigmoid,
        ]
    );
    assert_eq!((model.ops[0].act, model.ops[2].act), (Act::Relu6, Act::Relu));
    // The convolution's weights are the f16 values, its bias the f32 ones.
    assert_eq!(data(&model, model.ops[0].inputs[1]), &wv[..]);
    assert_eq!(data(&model, model.ops[0].inputs[2]), &bv[..]);
    assert_eq!(model.ops[2].inputs.len(), 2, "no bias (-1)");
    assert_eq!(model.ops[3].inputs.len(), 1, "the axes are not an input");
    assert_eq!(model.ops[10].inputs.len(), 3, "input, weights, bias");
    assert_eq!(sum.ops.iter().find(|o| o.0 == "LOGISTIC").map(|o| o.1), Some(2));
    // selfie.bin keeps it (f16 weights are exact).
    assert_eq!(Model::parse(&model.to_bytes()).unwrap(), model);
}

#[test]
fn builtin_transpose_conv_and_reshape() {
    let mut m = M::new();
    let x = m.act(&[1, 4, 5, 6]);
    let shape = m.ints(&[1, 8, 10, 3]);
    let (w, _) = m.weights(&[3, 3, 3, 6]);
    let (b, _) = m.weights(&[3]);
    let t = m.act(&[1, 8, 10, 3]);
    // (output_shape, weights, input, bias); SAME, stride 2, ReLU.
    m.op(67, "", &[shape, w, x, b], t, Some(Tab::default().f(0, F::U8(0)).f(1, F::I32(2)).f(2, F::I32(2)).f(3, F::I8(1))), None);
    let r = m.act(&[1, 80, 3]);
    let rs = m.ints(&[1, 80, 3]);
    m.op(22, "", &[t, rs], r, None, None);
    let file = m.file(x, r);
    let (model, _) = convert(&file).unwrap();
    // (3 - 1)·... SAME: (4 - 1)·2 + 3 - 8 = 1 → 0 before, 1 after.
    assert_eq!(model.ops[0].kind, Kind::TransposeConv { stride: [2, 2], pad: [0, 0] });
    assert_eq!(model.ops[0].act, Act::Relu);
    let ins = &model.ops[0].inputs;
    assert_eq!((model.tensors[ins[0] as usize].dims.clone(), model.tensors[ins[1] as usize].dims.clone()), (vec![1, 4, 5, 6], vec![3, 3, 3, 6]), "input first, then the weights");
    assert_eq!(model.ops[1].kind, Kind::Reshape);
    assert_eq!(model.tensors[model.output as usize].dims, [1, 80, 3]);
}

#[test]
fn what_it_cannot_convert_is_refused_by_name() {
    let err = |f: &dyn Fn(&mut M) -> (i32, i32)| {
        let mut m = M::new();
        let (i, o) = f(&mut m);
        convert(&m.file(i, o)).err().map(|e| e.0).unwrap_or_default()
    };
    let e = err(&|m| {
        let x = m.act(&[1, 4, 4, 2]);
        let y = m.act(&[1, 4, 4, 2]);
        m.op(25, "", &[x], y, None, None);
        (x, y)
    });
    assert!(e.contains("SOFTMAX is not supported"), "{e}");
    let e = err(&|m| {
        let x = m.act(&[1, 4, 4, 2]);
        let y = m.act(&[1, 4, 4, 2]);
        m.op(32, "MaxPoolingWithArgmax2D", &[x], y, None, None);
        (x, y)
    });
    assert!(e.contains("MaxPoolingWithArgmax2D is not supported"), "{e}");
    let e = err(&|m| {
        let x = m.act(&[1, 4, 4, 2]);
        let w = m.tensor(&[2, 1, 1, 2], U8T, vec![1, 2, 3, 4]);
        let y = m.act(&[1, 4, 4, 2]);
        m.op(3, "", &[x, w], y, Some(conv_opts(false, 1, 0)), None);
        (x, y)
    });
    assert!(e.contains("only float32 and float16"), "a quantized model: {e}");
    let e = err(&|m| {
        let x = m.act(&[1, 4, 4, 2]);
        let axes = m.ints(&[3]);
        let y = m.act(&[1, 4, 4, 1]);
        m.op(40, "", &[x, axes], y, None, None);
        (x, y)
    });
    assert!(e.contains("axes [3]"), "{e}");
    let e = err(&|m| {
        let x = m.act(&[1, 4, 4, 2]);
        let (w, _) = m.weights(&[2, 1, 1, 2]);
        let y = m.act(&[1, 4, 4, 2]);
        m.op(3, "", &[x, w], y, Some(conv_opts(false, 1, 5)), None);
        (x, y)
    });
    assert!(e.contains("fused activation 5"), "SIGN_BIT: {e}");
    assert!(convert(b"not a model at all").unwrap_err().0.contains("TFL3"));
}

/// A damaged file fails cleanly wherever it is cut or changed.
#[test]
fn damaged_files_fail_cleanly() {
    let mut m = M::new();
    let x = m.act(&[1, 6, 6, 3]);
    let (w, _) = m.weights_f16(&[4, 3, 3, 3]);
    let y = m.act(&[1, 3, 3, 4]);
    m.op(3, "", &[x, w], y, Some(conv_opts(false, 2, 0)), None);
    let file = m.file(x, y);
    assert!(convert(&file).is_ok());
    for n in 0..file.len() {
        let _ = convert(&file[..n]);
    }
    let mut s = 1u32;
    for _ in 0..20000 {
        s ^= s << 13;
        s ^= s >> 17;
        s ^= s << 5;
        let mut f = file.clone();
        let i = s as usize % f.len();
        f[i] = (s >> 8) as u8;
        let _ = convert(&f);
    }
}
